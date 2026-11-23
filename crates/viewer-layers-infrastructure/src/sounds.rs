
use viewer_layer_core::{layers::{Carried, Settling, Dirty, Inputs, Layer, List, Num, Scope, Scoped, Sight, Text, View}, touched::Touched};
use viewer_layer_core::playhead_host::mirror::{Mirror, Mirrors, UNREAD_REASON};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, NativeValue, Opaque, Recorded, TableId, TemplateGrids};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};
use viewer_audio::battle::{self, Battle, Battles, Front};

pub mod fed;
pub use fed::SoundsFed;

#[derive(Clone, Copy)]
struct Columns {
    center_x: Num<u32>,
    center_z: Num<u32>,
    radius: Num<u32>,
    provinces: List,
    object: Num<u64>,
    province: Num<u32>,
    attackers: List,
    defenders: List,
    layers: Option<Layered>,
}

#[derive(Clone, Copy)]
struct Layered {
    attacker_classes: List,
    defender_classes: List,
    template: Num<u64>,
    model_unreadable: Num<u8>,
    grids: List,
    literal: Text,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Combat {
    province: u32,
    units: usize,
    divisions: Box<[u64]>,
}

#[derive(Clone, Debug, PartialEq)]
struct Held {
    ordinal: u32,
    battle: Arc<Battle>,
    unknown: usize,
    made_at: u64,
}


const TABLES: [TableId; 5] = [TableId::BattleSounds, TableId::LandCombats, TableId::Divisions, TableId::DivisionTemplateIcons, TableId::LexicalTokens];

fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn note(mirror: Option<&Mirror>, name: &str) -> Option<String> {
    match mirror.and_then(Mirror::status) {
        Some(Status::Complete) => None,
        Some(Status::Truncated { .. }) => Some(format!("the recorded {name} table is incomplete")),
        Some(Status::Unavailable { .. }) => Some(format!("the recorded {name} table is unavailable ({})", mirror.and_then(Mirror::reason).unwrap_or(UNREAD_REASON))),
        Some(Status::NotRecorded) | None => Some(format!("the recorded {name} table is unavailable (not recorded at this hour)")),
    }
}

fn u64s(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
    bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().expect("eight bytes")))
}

fn u32s(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().expect("four bytes")))
}

fn list(mirror: &Mirror, column: List, slot: u32) -> &[u8] {
    mirror.list(column, slot).value().unwrap_or(&[])
}

pub struct Sounds {
    audio: Option<Arc<viewer_audio::Content>>,
    columns: Option<Columns>,
    combats: Vec<Option<Combat>>,
    by_province: HashMap<u32, Vec<u32>>,
    members: HashMap<u64, Vec<u32>>,
    fronts: HashMap<u32, Front<u32>>,
    battles: Vec<Option<Held>>,
    battle_of: HashMap<u32, Vec<u32>>,
    by_identity: HashMap<u64, Held>,
    changed_at: HashMap<u32, u64>,
    generation: u64,
    content_changed: bool,
    made: u64,
    compositions: HashMap<u64, Option<Vec<(u32, usize)>>>,
    names: Names,
    notes: Vec<String>,
    published: Arc<std::sync::Mutex<Option<Arc<Battles>>>>,
    mixer: Option<std::sync::mpsc::Sender<Mix>>,
    mix_inline: bool,
    last_notes: Option<Vec<String>>,
    combat_dirty: Dirty,
    province_dirty: Dirty,
    battle_dirty: Dirty,
    whole: bool,
    statuses: HashMap<usize, Option<Status>>,
    frame: u64,
    marked: [usize; 3],
    touched: Touched,
    scoped: Scoped,
    #[cfg(any(test, feature = "test-support"))]
    applied_reads: u64,
}

impl Sounds {
    pub fn new(audio: Option<Arc<viewer_audio::Content>>) -> Self {
        Self {
            audio,
            columns: None,
            combats: Vec::new(),
            by_province: HashMap::new(),
            members: HashMap::new(),
            fronts: HashMap::new(),
            battles: Vec::new(),
            battle_of: HashMap::new(),
            by_identity: HashMap::new(),
            changed_at: HashMap::new(),
            generation: 0,
            content_changed: true,
            made: 0,
            compositions: HashMap::new(),
            names: Names::default(),
            notes: Vec::new(),
            published: Arc::default(),
            mixer: None,
            mix_inline: false,
            last_notes: None,
            combat_dirty: Dirty::default(),
            province_dirty: Dirty::default(),
            battle_dirty: Dirty::default(),
            whole: true,
            statuses: HashMap::new(),
            frame: 0,
            marked: [0; 3],
            touched: Touched::default(),
            scoped: Scoped::default(),
            #[cfg(any(test, feature = "test-support"))]
            applied_reads: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new(self.audio.clone()) }
    }

    fn drawn(&self) -> bool {
        self.audio.as_ref().is_some_and(|audio| audio.battle.is_some())
    }

    fn sub_units(&self) -> bool {
        self.audio.as_ref().is_some_and(|audio| !audio.layers.sounds.is_empty())
    }

    pub fn product(&self) -> Option<Battles> {
        self.columns?;
        let mut held: Vec<&Held> = self.battles.iter().filter_map(Option::as_ref).collect();
        held.sort_unstable_by_key(|held| held.ordinal);
        let mut refusals = self.notes.clone();
        refusals.extend(battle::unknown_refusal(held.iter().map(|held| held.unknown).sum()));
        Some(Battles { battles: held.into_iter().map(|held| (*held.battle).clone()).collect(), refusals })
    }

    fn read_notes(&mut self, mirrors: &Mirrors) {
        let Some(columns) = self.columns else { return };
        let read: &[TableId] = if columns.layers.is_some() { &TABLES } else { &TABLES[..2] };
        self.notes.clear();
        self.notes.extend(read.iter().filter_map(|table| note(mirrors.get(*table), &format!("{table:?}"))));
    }

    fn publish(&mut self, notes: Vec<String>) {
        self.last_notes = Some(notes.clone());
        let kept = || self.battles.iter().filter_map(Option::as_ref);
        let battles: Vec<(u32, Arc<Battle>)> = kept().map(|held| (held.ordinal, Arc::clone(&held.battle))).collect();
        let unknown = kept().map(|held| held.unknown).sum();
        let job = Mix { battles, notes, unknown, drawn: self.columns.is_some() };
        if self.mix_inline {
            *self.published.lock().unwrap_or_else(|p| p.into_inner()) = job.make();
            return;
        }
        let jobs = self.mixer.get_or_insert_with(|| {
            let (jobs, inbox) = std::sync::mpsc::channel::<Mix>();
            let out = Arc::clone(&self.published);
            let _ = std::thread::Builder::new().name("battle mix".into()).spawn(move || {
                viewer_runtime::worker_priority::background_priority();
                while let Ok(mut job) = inbox.recv() {
                    while let Ok(newer) = inbox.try_recv() {
                        job = newer;
                    }
                    let mixed = job.make();
                    *out.lock().unwrap_or_else(|p| p.into_inner()) = mixed;
                }
            });
            jobs
        });
        let _ = jobs.send(job);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn marked(&self) -> [usize; 3] {
        self.marked
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    fn mark_combat(&mut self, slot: u32) {
        if self.combat_dirty.mark(slot, self.frame) {
            self.marked[0] += 1;
        }
    }

    fn mark_province(&mut self, province: u32) {
        if self.province_dirty.mark(province, self.frame) {
            self.marked[1] += 1;
        }
    }

    fn mark_battle(&mut self, slot: u32) {
        if self.battle_dirty.mark(slot, self.frame) {
            self.marked[2] += 1;
        }
    }

    fn mark_members(&mut self, mirrors: &Mirrors, wanted: impl Fn(Option<u64>) -> bool) {
        let Some(layered) = self.columns.and_then(|columns| columns.layers) else { return };
        let divisions = rows(mirrors.get(TableId::Divisions));
        let template_of = |division: u64| divisions.and_then(|mirror| mirror.slot(u128::from(division))).and_then(|slot| divisions?.number(layered.template, slot).value());
        let slots: BTreeSet<u32> = self.members.iter().filter(|(division, _)| wanted(template_of(**division))).flat_map(|(_, slots)| slots.iter().copied()).collect();
        self.mark_fronts_of(slots);
    }

    fn mark_fronts_of(&mut self, slots: BTreeSet<u32>) {
        let provinces: BTreeSet<u32> = slots.into_iter().filter_map(|slot| self.combats.get(slot as usize)?.as_ref().map(|combat| combat.province)).collect();
        provinces.into_iter().for_each(|province| self.mark_province(province));
    }

    fn composition(compositions: &mut HashMap<u64, Option<Vec<(u32, usize)>>>, names: &mut Names, mirrors: &Mirrors, layered: Layered, division: u64) -> Option<Vec<(u32, usize)>> {
        let divisions = rows(mirrors.get(TableId::Divisions))?;
        let template = divisions.number(layered.template, divisions.slot(u128::from(division))?).value()?;
        if template == 0 {
            return Some(Vec::new());
        }
        compositions
            .entry(template)
            .or_insert_with(|| {
                let templates = rows(mirrors.get(TableId::DivisionTemplateIcons))?;
                let slot = templates.slot(u128::from(template))?;
                if templates.number(layered.model_unreadable, slot).value() != Some(0) {
                    return None;
                }
                let bytes = templates.list(layered.grids, slot).value()?;
                let Ok(Recorded::Value(grids)) = Recorded::<TemplateGrids>::from_field(Field::Opaque(Opaque::Encoded(bytes, templates.format()))) else { return None };
                let tokens = rows(mirrors.get(TableId::LexicalTokens));
                let name = |token: u32| -> Option<u32> {
                    let tokens = tokens?;
                    let id = tokens.text(layered.literal, tokens.slot(u128::from(token))?).value()?;
                    Some(names.id(tokens.string(id)?))
                };
                battle::composition(grids.ambience_slots().map(|subunit| (subunit.token, subunit.count)), name)
            })
            .clone()
    }

    fn read_combat(&mut self, slot: u32, mirrors: &Mirrors) {
        let columns = self.columns.expect("declared before it settles");
        let at = slot as usize;
        let now = rows(mirrors.get(TableId::LandCombats)).filter(|mirror| mirror.live(slot)).and_then(|mirror| {
            let province = mirror.number(columns.province, slot).value()?;
            let (attackers, defenders) = (list(mirror, columns.attackers, slot), list(mirror, columns.defenders, slot));
            let divisions = match columns.layers {
                Some(layered) => {
                    let (attacker_classes, defender_classes) = (list(mirror, layered.attacker_classes, slot), list(mirror, layered.defender_classes, slot));
                    let (attackers, defenders): (Vec<u64>, Vec<u64>) = (u64s(attackers).collect(), u64s(defenders).collect());
                    let (attacker_classes, defender_classes): (Vec<u32>, Vec<u32>) = (u32s(attacker_classes).collect(), u32s(defender_classes).collect());
                    battle::divisions(&attackers, &attacker_classes, &defenders, &defender_classes).collect()
                }
                None => Box::default(),
            };
            Some(Combat { province, units: attackers.len() / 8 + defenders.len() / 8, divisions })
        });
        if self.combats.len() <= at {
            self.combats.resize(at + 1, None);
        }
        if self.combats[at] == now {
            return;
        }
        if let Some(old) = self.combats[at].take() {
            unlink(&mut self.by_province, old.province, slot);
            old.divisions.iter().for_each(|&division| unlink(&mut self.members, division, slot));
            self.mark_province(old.province);
        }
        if let Some(new) = &now {
            self.by_province.entry(new.province).or_default().push(slot);
            new.divisions.iter().for_each(|&division| self.members.entry(division).or_default().push(slot));
            self.mark_province(new.province);
        }
        self.combats[at] = now;
    }

    fn read_front(&mut self, province: u32, mirrors: &Mirrors) {
        let mut front = Front::default();
        if let Some(slots) = self.by_province.get(&province) {
            let layered = self.columns.and_then(|columns| columns.layers);
            for &slot in slots {
                let Some(combat) = &self.combats[slot as usize] else { continue };
                let (compositions, names) = (&mut self.compositions, &mut self.names);
                front.add(combat.units, combat.divisions.iter().copied(), layered.is_some(), |division| Self::composition(compositions, names, mirrors, layered.expect("layers read"), division));
            }
        }
        let changed = match self.fronts.get(&province) {
            Some(held) => *held != front,
            None => front != Front::default() || self.by_province.contains_key(&province),
        };
        if self.by_province.contains_key(&province) {
            self.fronts.insert(province, front);
        } else {
            self.fronts.remove(&province);
        }
        if changed {
            self.generation += 1;
            self.changed_at.insert(province, self.generation);
            let battles: Vec<u32> = self.battle_of.get(&province).cloned().unwrap_or_default();
            battles.into_iter().for_each(|slot| self.mark_battle(slot));
        }
    }

    fn read_battle(&mut self, slot: u32, mirrors: &Mirrors) {
        let columns = self.columns.expect("declared before it settles");
        let at = slot as usize;
        let now = rows(mirrors.get(TableId::BattleSounds)).filter(|mirror| mirror.live(slot)).and_then(|mirror| {
            let ordinal = mirror.key(slot)? as u32;
            let number = |column: Num<u32>| mirror.number(column, slot).value().unwrap_or(0);
            let provinces: Vec<u32> = u32s(list(mirror, columns.provinces, slot)).collect();
            let identity = identity(mirror, columns, slot);
            let (center, radius) = ([f32::from_bits(number(columns.center_x)), f32::from_bits(number(columns.center_z))], f32::from_bits(number(columns.radius)));
            let changed_at = &self.changed_at;
            if let Some(held) = self.by_identity.get(&identity)
                && held.battle.provinces == provinces
                && held.battle.center.map(f32::to_bits) == center.map(f32::to_bits)
                && held.battle.radius.to_bits() == radius.to_bits()
                && provinces.iter().all(|province| changed_at.get(province).is_none_or(|&at| at <= held.made_at))
            {
                return Some(Held { ordinal, ..held.clone() });
            }
            let mut unknown = 0;
            let (fronts, names) = (&self.fronts, &self.names);
            let battle = Arc::new(battle::battle(identity, provinces, center, radius, |province| fronts.get(&province), |id| names.name(*id), &mut unknown));
            self.content_changed = true;
            self.made += 1;
            let held = Held { ordinal, battle, unknown, made_at: self.generation };
            self.by_identity.insert(identity, held.clone());
            Some(held)
        });
        if now.is_none() && self.battles.get(at).is_some_and(Option::is_some) {
            self.content_changed = true;
        }
        if self.battles.len() <= at {
            self.battles.resize(at + 1, None);
        }
        let old: BTreeSet<u32> = self.battles[at].as_ref().map(|held| held.battle.provinces.iter().copied().collect()).unwrap_or_default();
        let new: BTreeSet<u32> = now.as_ref().map(|held| held.battle.provinces.iter().copied().collect()).unwrap_or_default();
        for &province in old.difference(&new) {
            unlink(&mut self.battle_of, province, slot);
        }
        for &province in new.difference(&old) {
            self.battle_of.entry(province).or_default().push(slot);
        }
        self.battles[at] = now;
    }

    fn mark_all(&mut self, mirrors: &Mirrors) {
        self.compositions.clear();
        self.statuses = TABLES.iter().map(|table| (table.index(), mirrors.get(*table).and_then(Mirror::status))).collect();
        let combats: Vec<u32> = (0..self.combats.len() as u32).filter(|&slot| self.combats[slot as usize].is_some()).chain(rows(mirrors.get(TableId::LandCombats)).into_iter().flat_map(Mirror::rows)).collect();
        combats.into_iter().for_each(|slot| self.mark_combat(slot));
        let provinces: Vec<u32> = self.by_province.keys().chain(self.fronts.keys()).copied().collect();
        provinces.into_iter().for_each(|province| self.mark_province(province));
        let battles: Vec<u32> = (0..self.battles.len() as u32).filter(|&slot| self.battles[slot as usize].is_some()).chain(rows(mirrors.get(TableId::BattleSounds)).into_iter().flat_map(Mirror::rows)).collect();
        battles.into_iter().for_each(|slot| self.mark_battle(slot));
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        if self.columns.is_none() {
            return;
        }
        if std::mem::take(&mut self.whole) {
            self.touched.clear();
            self.mark_all(mirrors);
        }
        self.take_touched(mirrors);
        self.read_notes(mirrors);
        let mut dirty = std::mem::take(&mut self.combat_dirty);
        let _ = dirty.settle(|slot| {
            self.read_combat(slot, mirrors);
            match self.combats.get(slot as usize).and_then(Option::as_ref) {
                Some(combat) => self.scoped.heard_province(combat.province),
                None => self.scoped.unplaced(),
            }
            Ok(())
        });
        self.combat_dirty = dirty;
        let mut dirty = std::mem::take(&mut self.province_dirty);
        let _ = dirty.settle(|province| {
            self.scoped.heard_province(province);
            self.read_front(province, mirrors);
            Ok(())
        });
        self.province_dirty = dirty;
        let notes = self.notes.clone();
        let mut dirty = std::mem::take(&mut self.battle_dirty);
        let _ = dirty.settle(|slot| {
            self.read_battle(slot, mirrors);
            match self.battles.get(slot as usize).and_then(Option::as_ref) {
                Some(held) => self.scoped.heard(held.battle.center, held.battle.radius),
                None => self.scoped.unplaced(),
            }
            Ok(())
        });
        let changed = self.content_changed || self.last_notes.as_ref() != Some(&notes);
        self.battle_dirty = dirty;
        if changed && self.combat_dirty.len() + self.province_dirty.len() + self.battle_dirty.len() == 0 {
            self.content_changed = false;
            let live: std::collections::HashSet<u64> = self.battles.iter().flatten().map(|held| held.battle.key).collect();
            self.by_identity.retain(|identity, _| live.contains(identity));
            self.publish(notes);
        }
    }

    fn apply_step(&mut self, step: &Step) {
        let Some(columns) = self.columns else { return };
        if step.gap() {
            self.whole = true;
            return;
        }
        let mut order = TABLES;
        order.sort_unstable_by_key(|table| table.index());
        for table in order {
            let Some(changes) = step.table(table.index()) else { continue };
            if let Some((status, _)) = changes.status()
                && self.statuses.insert(changes.table(), Some(status)) != Some(Some(status))
            {
                self.whole = true;
            }
            if self.whole {
                continue;
            }
            match table {
                TableId::LandCombats => {
                    let mut read = vec![columns.province.column, columns.attackers.column, columns.defenders.column];
                    if let Some(layered) = columns.layers {
                        read.extend([layered.attacker_classes.column, layered.defender_classes.column]);
                    }
                    for &slot in changes.deleted().slots().iter().chain(changes.inserted().slots()) {
                        self.mark_combat(slot);
                    }
                    for slot in written(changes, &read) {
                        self.mark_combat(slot);
                    }
                }
                TableId::BattleSounds => {
                    let read = [columns.center_x.column, columns.center_z.column, columns.radius.column, columns.provinces.column, columns.object.column];
                    for &slot in changes.deleted().slots().iter().chain(changes.inserted().slots()) {
                        self.mark_battle(slot);
                    }
                    for slot in written(changes, &read) {
                        self.mark_battle(slot);
                    }
                }
                TableId::Divisions | TableId::DivisionTemplateIcons | TableId::LexicalTokens => {
                    let Some(layered) = columns.layers else { continue };
                    let read: &[usize] = match table {
                        TableId::Divisions => &[layered.template.column],
                        TableId::DivisionTemplateIcons => &[layered.model_unreadable.column, layered.grids.column],
                        _ => &[layered.literal.column],
                    };
                    self.touched.note(changes, self.frame, |column| u8::from(read.contains(&column)));
                }
                _ => {}
            }
        }
    }

    fn take_touched(&mut self, mirrors: &Mirrors) {
        let mut touched = std::mem::take(&mut self.touched);
        let (mut divisions, mut templates, mut tokens) = (BTreeSet::new(), BTreeSet::new(), false);
        touched.take(mirrors, |table, _, key| match table {
            TableId::Divisions => {
                divisions.insert(key as u64);
            }
            TableId::DivisionTemplateIcons => {
                templates.insert(key as u64);
            }
            _ => tokens = true,
        });
        self.touched = touched;
        let slots: BTreeSet<u32> = divisions.iter().filter_map(|division| self.members.get(division)).flatten().copied().collect();
        self.mark_fronts_of(slots);
        if !templates.is_empty() {
            templates.iter().for_each(|template| {
                self.compositions.remove(template);
            });
            self.mark_members(mirrors, |template| template.is_some_and(|template| templates.contains(&template)));
        }
        if tokens {
            self.compositions.clear();
            self.mark_members(mirrors, |_| true);
        }
    }
}

pub fn drawn(side: &viewer_layer_core::playhead_host::MapSide<impl viewer_layer_core::playhead_host::PlaybackProducts>) -> Option<Option<Arc<Battles>>> {
    if !side.draws("sounds") {
        return None;
    }
    let (_, layers) = side.parts()?;
    layers.iter().find_map(|layer| layer.any().downcast_ref::<SoundsFed>()).map(|layer| layer.battles().cloned())
}

struct Mix {
    battles: Vec<(u32, Arc<Battle>)>,
    notes: Vec<String>,
    unknown: usize,
    drawn: bool,
}

impl Mix {
    fn make(mut self) -> Option<Arc<Battles>> {
        if !self.drawn {
            return None;
        }
        self.battles.sort_unstable_by_key(|(ordinal, _)| *ordinal);
        let mut refusals = self.notes;
        refusals.extend(battle::unknown_refusal(self.unknown));
        Some(Arc::new(Battles { battles: self.battles.into_iter().map(|(_, battle)| (*battle).clone()).collect(), refusals }))
    }
}

fn identity(mirror: &Mirror, columns: Columns, slot: u32) -> u64 {
    mirror.number(columns.object, slot).value().unwrap_or(0)
}

pub fn differ_battles(a: Option<&Battles>, b: Option<&Battles>) -> (u64, Option<String>) {
    let mut differing = 0;
    let mut first = None;
    let mut note = |what: String| {
        differing += 1;
        first.get_or_insert(what);
    };
    match (a, b) {
        (None, None) => {}
        (Some(_), None) | (None, Some(_)) => note(format!("one draws battles, the other none: {:?} against {:?}", a.is_some(), b.is_some())),
        (Some(a), Some(b)) => {
            if a.battles.len() != b.battles.len() {
                note(format!("{} battles against {}", a.battles.len(), b.battles.len()));
            }
            for (at, (x, y)) in a.battles.iter().zip(&b.battles).enumerate() {
                let same = x.key == y.key && x.provinces == y.provinces && x.center.map(f32::to_bits) == y.center.map(f32::to_bits) && x.radius.to_bits() == y.radius.to_bits() && x.divisions == y.divisions && x.sub_units == y.sub_units;
                if !same {
                    note(format!("battle {at}: {x:?} against {y:?}"));
                }
            }
            if a.refusals != b.refusals {
                note(format!("refusals: {:?} against {:?}", a.refusals, b.refusals));
            }
        }
    }
    (differing, first)
}

fn written<'a>(changes: &'a Changes, wanted: &'a [usize]) -> impl Iterator<Item = u32> + 'a {
    changes.columns().iter().filter(|written| wanted.contains(&written.column())).flat_map(|written| written.slots().iter().copied())
}

#[derive(Default)]
struct Names {
    names: Vec<String>,
    ids: HashMap<String, u32>,
}

impl Names {
    fn id(&mut self, bytes: &[u8]) -> u32 {
        let name = String::from_utf8_lossy(bytes);
        if let Some(&id) = self.ids.get(name.as_ref()) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.clone().into_owned());
        self.ids.insert(name.into_owned(), id);
        id
    }

    fn name(&self, id: u32) -> String {
        self.names[id as usize].clone()
    }

    fn bytes(&self) -> usize {
        self.names.iter().map(|name| name.capacity() * 2 + 64).sum()
    }
}

fn unlink<K: std::hash::Hash + Eq>(index: &mut HashMap<K, Vec<u32>>, key: K, slot: u32) {
    if let Some(slots) = index.get_mut(&key) {
        if let Some(at) = slots.iter().position(|&held| held == slot) {
            slots.swap_remove(at);
        }
        if slots.is_empty() {
            index.remove(&key);
        }
    }
}

impl Layer for Sounds {
    fn name(&self) -> &'static str {
        "sounds"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.whole = true;
        let audible = view.sound && [view.audio.master, view.audio.ambient, view.audio.combat].iter().all(|volume| *volume > 0.0);
        if !self.drawn() || !audible {
            self.columns = None;
            return Ok(());
        }
        inputs.table(TableId::BattleSounds, true)?;
        inputs.table(TableId::LandCombats, false)?;
        let layers = if self.sub_units() {
            for table in [TableId::Divisions, TableId::DivisionTemplateIcons, TableId::LexicalTokens] {
                inputs.table(table, true)?;
            }
            Some(Layered {
                attacker_classes: inputs.list(TableId::LandCombats, "attacker_classes")?,
                defender_classes: inputs.list(TableId::LandCombats, "defender_classes")?,
                template: inputs.number(TableId::Divisions, "template")?,
                model_unreadable: inputs.number(TableId::DivisionTemplateIcons, "model_unreadable")?,
                grids: inputs.list(TableId::DivisionTemplateIcons, "grids")?,
                literal: inputs.text(TableId::LexicalTokens, "literal")?,
            })
        } else {
            None
        };
        self.columns = Some(Columns {
            center_x: inputs.number(TableId::BattleSounds, "center_x")?,
            center_z: inputs.number(TableId::BattleSounds, "center_z")?,
            radius: inputs.number(TableId::BattleSounds, "radius")?,
            provinces: inputs.list(TableId::BattleSounds, "provinces")?,
            object: inputs.number(TableId::BattleSounds, "object")?,
            province: inputs.number(TableId::LandCombats, "province")?,
            attackers: inputs.list(TableId::LandCombats, "attackers")?,
            defenders: inputs.list(TableId::LandCombats, "defenders")?,
            layers,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.whole = true;
        let inline = std::mem::replace(&mut self.mix_inline, true);
        self.settle_dirty(mirrors);
        self.mix_inline = inline;
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        #[cfg(any(test, feature = "test-support"))]
        let reads = viewer_layer_core::playhead_host::mirror::probe::reads();
        self.apply_step(step);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads += viewer_layer_core::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }

    fn carried(&self) -> Carried {
        let entities = self.combat_dirty.len() + self.province_dirty.len() + self.battle_dirty.len() + self.touched.len() + usize::from(self.whole && self.columns.is_some());
        Carried { entities }
    }

    fn scope(&mut self, sight: &Sight<'_>) -> Option<Scope> {
        let hearing = self.audio.as_deref().and_then(battle::hearing);
        Some(self.scoped.take(sight, hearing))
    }

    fn held(&self) -> usize {
        let index = |index: &HashMap<u32, Vec<u32>>| index.capacity() * 32 + index.values().map(|slots| slots.capacity() * 4).sum::<usize>();
        let battle = |held: &Held| held.battle.provinces.capacity() * 4 + held.battle.sub_units.iter().map(|(name, _)| name.capacity() + 48).sum::<usize>();
        self.combats.capacity() * size_of::<Option<Combat>>()
            + self.combats.iter().flatten().map(|combat| combat.divisions.len() * 8).sum::<usize>()
            + index(&self.by_province)
            + self.members.capacity() * 40
            + self.members.values().map(|slots| slots.capacity() * 4).sum::<usize>()
            + self.fronts.capacity() * (8 + size_of::<Front<u32>>())
            + self.fronts.values().map(|front| front.sub_units.len() * 40).sum::<usize>()
            + self.names.bytes()
            + self.battles.capacity() * size_of::<Option<Held>>()
            + self.battles.iter().flatten().map(battle).sum::<usize>()
            + index(&self.battle_of)
            + self.changed_at.capacity() * 24
            + self.compositions.capacity() * 40
            + self.compositions.values().flatten().map(|units| units.capacity() * 16).sum::<usize>()
            + self.combat_dirty.bytes()
            + self.province_dirty.bytes()
            + self.battle_dirty.bytes()
            + self.touched.bytes()
            + self.scoped.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
