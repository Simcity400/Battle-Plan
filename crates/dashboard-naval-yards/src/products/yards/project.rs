//! A yard view's rows over one segment's hours ([`super::layout`]): built whole at the segment's
//! first hour, then projected again only where an hour's changed rows reach, each port, line,
//! ship and owner's totals kept between hours; an hour that restates a table, or the countries
//! and spellings everything is named by, projects the view whole again. An hour hands on the rows
//! it changed.
use super::{
    layout::{self, line, port, refit_totals, repair_totals},
    *,
};
use dashboard_base::prepared::prepass::{Held, HeldBytes};
use dashboard_keyed::segment::{Touch, Touched};
use labels::{Naming, Reasons, Shared, label};
use std::collections::HashSet;
pub(crate) use labels::Label;
use refits::Refits;
use repairs::Repairs;
use ship::Ship;

mod labels;
mod refits;
mod repairs;
mod ship;

/// A cell as a reader hands it on: a label by its operand, which only the branch numbers.
#[derive(Clone, Debug)]
#[derive(replay_spool::Spooled)]
pub(crate) enum Spelled {
    Null,
    Integer(i128),
    Unsigned(u64),
    Token(Arc<Label>),
    List(Vec<Spelled>),
}

impl PartialEq for Spelled {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Unsigned(a), Self::Unsigned(b)) => a == b,
            (Self::Token(a), Self::Token(b)) => Arc::ptr_eq(a, b) || a.key == b.key,
            (Self::List(a), Self::List(b)) => a == b,
            _ => false,
        }
    }
}

impl Spelled {
    /// What it holds that the hour carrying it is charged for.
    fn carry(&self) -> usize {
        match self {
            Spelled::Token(label) => label.carry(),
            Spelled::List(cells) => {
                cells.capacity() * size_of::<Spelled>()
                    + cells.iter().map(Spelled::carry).sum::<usize>()
            }
            Spelled::Null | Spelled::Integer(_) | Spelled::Unsigned(_) => 0,
        }
    }
}

impl std::fmt::Debug for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)
    }
}

/// A row's key: a number, or a refusal's reason.
#[derive(Clone, Debug)]
#[derive(replay_spool::Spooled)]
pub(crate) enum Key {
    Number(u64),
    Token(Arc<Label>),
}

/// A row as a reader hands it on: every cell after its key and whether it is held, or none
/// where the hour removed it.
#[derive(Debug)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Row {
    /// Its owner, by its number in the hour's owners.
    pub(crate) owner: u32,
    pub(crate) stream: u64,
    pub(crate) key: Key,
    pub(crate) cells: Option<Vec<Spelled>>,
}

impl Row {
    /// What it holds that the hour carrying it is charged for.
    pub(crate) fn carry(&self) -> usize {
        let key = match &self.key {
            Key::Token(label) => label.carry(),
            Key::Number(_) => 0,
        };
        key + self.cells.as_ref().map_or(0, |cells| {
            cells.capacity() * size_of::<Spelled>() + cells.iter().map(Spelled::carry).sum::<usize>()
        })
    }
}

/// The rows an hour changed, and the owners they are of; every row the view holds where whole.
#[derive(replay_spool::Spooled)]
pub(crate) struct Rows {
    pub(crate) owners: Arc<[String]>,
    pub(crate) whole: bool,
    pub(crate) rows: Vec<Row>,
}

/// Where a row is: its owner, stream and key; a refusal's key is its reason's number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Place {
    owner: u32,
    stream: u64,
    key: u64,
}

/// What an hour's changes reach: rows, and owners whose totals read them.
#[derive(Default)]
pub(super) struct Dirty {
    places: HashSet<Place>,
    totals: HashSet<u32>,
    /// Every owner's totals.
    totals_all: bool,
}

impl Dirty {
    fn place(&mut self, owner: u32, stream: u64, key: u64) {
        self.places.insert(Place { owner, stream, key });
    }

    fn totals(&mut self, owner: u32) {
        self.totals.insert(owner);
    }
}

/// Each owner's refusal counts, by reason.
#[derive(Default)]
pub(super) struct Refusals {
    counts: HashMap<(u32, u32), u64>,
}

impl Refusals {
    /// `owner`'s counts from `old` to `now`.
    fn shift(&mut self, dirty: &mut Dirty, owner: u32, old: &[(u32, u64)], now: &[(u32, u64)]) {
        if old == now {
            return;
        }
        for &(reason, count) in old {
            let held = self.counts.entry((owner, reason)).or_default();
            *held -= count;
            if *held == 0 {
                self.counts.remove(&(owner, reason));
            }
            dirty.place(owner, layout::REFUSALS, u64::from(reason));
        }
        for &(reason, count) in now {
            *self.counts.entry((owner, reason)).or_default() += count;
            dirty.place(owner, layout::REFUSALS, u64::from(reason));
        }
    }
}

/// The owners an hour's rows name, numbered in the order the segment first names them.
#[derive(Default)]
struct Owners {
    names: Vec<String>,
    numbers: HashMap<String, u32>,
    /// The names as hours carry them, made again when one is added; whether one carried them.
    shared: Option<(Arc<[String]>, bool)>,
}

impl Owners {
    fn number(&mut self, name: String) -> u32 {
        if let Some(&number) = self.numbers.get(&name) {
            return number;
        }
        let number = self.names.len() as u32;
        self.names.push(name.clone());
        self.numbers.insert(name, number);
        self.shared = None;
        number
    }

    fn shared(&mut self) -> (Arc<[String]>, bool) {
        let (names, carried) = self
            .shared
            .get_or_insert_with(|| (self.names.clone().into(), false));
        (names.clone(), !std::mem::replace(carried, true))
    }

    /// The names as hours carry them, without marking them carried.
    fn names(&mut self) -> Arc<[String]> {
        self.shared
            .get_or_insert_with(|| (self.names.clone().into(), false))
            .0
            .clone()
    }
}

/// Who the hour's countries are: each canonical slot's owner, each owner's raw references, and
/// the owners its countries name.
struct Identity {
    countries: Countries,
    slots: HashMap<u32, u32>,
    raws: HashMap<u32, Vec<u32>>,
    current: Vec<u32>,
}

impl Identity {
    /// None where the hour's countries or their links are not complete.
    fn read(frame: &DecodedFrame, owners: &mut Owners) -> Option<Self> {
        let countries = Countries::read(frame)?;
        let mut current: Vec<u32> = super::owners(frame, &countries)
            .into_iter()
            .map(|name| owners.number(name))
            .collect();
        current.sort_unstable();
        let mut slots = HashMap::new();
        for row in frame.complete::<tables::Countries>().ok()? {
            if let Some(name) = countries.slotted(row.canonical_slot) {
                slots.insert(row.canonical_slot, owners.number(name));
            }
        }
        let mut raws: HashMap<u32, Vec<u32>> = HashMap::new();
        for link in frame.complete::<tables::CountryLinks>().ok()? {
            if let Some(owner) = countries.slot(link.id).and_then(|slot| slots.get(&slot)) {
                raws.entry(*owner).or_default().push(link.id);
            }
        }
        for held in raws.values_mut() {
            held.sort_unstable();
            held.dedup();
        }
        Some(Self {
            countries,
            slots,
            raws,
            current,
        })
    }
}

/// What a projection reads at an hour.
struct Cx<'f, 's> {
    frame: &'f DecodedFrame,
    countries: &'s Countries,
    identity: &'s Identity,
    shared: &'s mut Shared,
    naming: Naming<'f>,
    reasons: &'s mut Reasons,
    refusals: &'s mut Refusals,
    /// The Ships field strength is.
    strength: u64,
}

impl Cx<'_, '_> {
    /// The owner a raw country reference resolves to.
    fn owner(&self, raw: u32) -> Option<u32> {
        let slot = self.countries.slot(raw)?;
        self.identity.slots.get(&slot).copied()
    }

    /// The raw references resolving to `owner`, ascending.
    fn raws(&self, owner: u32) -> &[u32] {
        self.identity.raws.get(&owner).map_or(&[], Vec::as_slice)
    }
}

/// Every ship a port or line holds, and the ships each task force and variant reaches.
#[derive(Default)]
pub(super) struct Ships {
    ships: HashMap<u64, Ship>,
    tasks: HashMap<u64, Vec<u64>>,
    variants: HashMap<u64, Vec<u64>>,
    classes: HashMap<u32, Vec<u64>>,
}

impl Ships {
    fn index(&mut self, id: u64, ship: &Ship) {
        for &task in &ship.tasks {
            self.tasks.entry(task).or_default().push(id);
        }
        for &variant in &ship.variants {
            self.variants.entry(variant).or_default().push(id);
        }
        if let Some(class) = ship.class {
            self.classes.entry(class).or_default().push(id);
        }
    }

    fn unindex(&mut self, id: u64, ship: &Ship) {
        for &task in &ship.tasks {
            remove(&mut self.tasks, task, id);
        }
        for &variant in &ship.variants {
            remove(&mut self.variants, variant, id);
        }
        if let Some(class) = ship.class {
            remove(&mut self.classes, class, id);
        }
    }

    /// What `holder` held, `before`, and holds, `after`: an owner and its ships, ascending.
    fn rehold(
        &mut self,
        cx: &mut Cx<'_, '_>,
        dirty: &mut Dirty,
        holder: u64,
        before: Option<(u32, Vec<u64>)>,
        after: Option<(u32, Vec<u64>)>,
    ) {
        if before == after {
            return;
        }
        let same = |side: &Option<(u32, Vec<u64>)>, owner: u32, ship: &u64| {
            side.as_ref()
                .is_some_and(|(held, ships)| *held == owner && ships.binary_search(ship).is_ok())
        };
        if let Some((owner, ids)) = &before {
            for id in ids.iter().filter(|id| !same(&after, *owner, id)) {
                let Some(held) = self.ships.get_mut(id) else {
                    continue;
                };
                if let Ok(at) = held.holders.binary_search(&(*owner, holder)) {
                    held.holders.remove(at);
                }
                cx.refusals.shift(dirty, *owner, &held.refusals, &[]);
                dirty.place(*owner, layout::SHIPS, *id);
                if held.holders.is_empty()
                    && let Some(gone) = self.ships.remove(id)
                {
                    self.unindex(*id, &gone);
                }
            }
        }
        if let Some((owner, ids)) = &after {
            for id in ids.iter().filter(|id| !same(&before, *owner, id)) {
                if !self.ships.contains_key(id) {
                    let projected = ship::project(cx, *id, None);
                    self.index(*id, &projected);
                    self.ships.insert(*id, projected);
                }
                let held = self.ships.get_mut(id).expect("a ship just held");
                if let Err(at) = held.holders.binary_search(&(*owner, holder)) {
                    held.holders.insert(at, (*owner, holder));
                }
                cx.refusals.shift(dirty, *owner, &[], &held.refusals);
                dirty.place(*owner, layout::SHIPS, *id);
            }
        }
    }

    /// The ship `id` projected again where a port or line holds it.
    fn project(&mut self, cx: &mut Cx<'_, '_>, dirty: &mut Dirty, id: u64) {
        let Some(old) = self.ships.remove(&id) else {
            return;
        };
        self.unindex(id, &old);
        let mut now = ship::project(cx, id, Some(&old.cells));
        now.holders = old.holders;
        for &(owner, _) in &now.holders {
            cx.refusals.shift(dirty, owner, &old.refusals, &now.refusals);
            dirty.place(owner, layout::SHIPS, id);
        }
        self.index(id, &now);
        self.ships.insert(id, now);
    }

    /// The ship `id`'s hull, where a port or line holds it and the hour wrote only its strength.
    fn strengthen(&mut self, frame: &DecodedFrame, dirty: &mut Dirty, id: u64) {
        if let Some(held) = self.ships.get_mut(&id) {
            held.strengthened(frame, id);
            for &(owner, _) in &held.holders {
                dirty.place(owner, layout::SHIPS, id);
            }
        }
    }

    /// Each owner holding the ship `id`.
    fn owners_of(&self, id: u64, mut each: impl FnMut(u32)) {
        if let Some(held) = self.ships.get(&id) {
            for &(owner, _) in &held.holders {
                each(owner);
            }
        }
    }

    /// `owner`'s row of the ship `id`.
    fn row(&self, owner: u32, id: u64) -> Option<Vec<Spelled>> {
        let held = self.ships.get(&id)?;
        let holder = held.holder(owner)?;
        let mut cells = held.cells.to_vec();
        cells[layout::ship::HOLDER - 2] = Spelled::Unsigned(holder);
        Some(cells)
    }
}

enum View {
    Repairs(Repairs),
    Refits(Refits),
}

/// What a projection holds of the hour, made again where it is projected whole.
struct State {
    identity: Identity,
    shared: Shared,
    refusals: Refusals,
    ships: Ships,
    view: View,
    totals: HashMap<u32, Box<[Spelled]>>,
}

/// Tables everything a view holds is named or owned by: an hour changing them projects it whole.
const NAMING: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
];

/// Tables only one view reads.
const REFITS_ONLY: &[TableId] = &[
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::ProductionLines,
];
const REPAIRS_ONLY: &[TableId] = &[
    TableId::NavalRepairCountries,
    TableId::CountryFactories,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::Provinces,
    TableId::States,
    TableId::ProvinceNames,
];

fn reads(table: TableId, refits: bool) -> bool {
    !(if refits { REPAIRS_ONLY } else { REFITS_ONLY }).contains(&table)
}

/// A yard view's projection of one segment.
pub(crate) struct Projector {
    refits: bool,
    strength: u64,
    owners: Owners,
    reasons: Reasons,
    state: State,
    /// Each row as last handed on.
    published: HashMap<Place, Box<[Spelled]>>,
}

impl Projector {
    /// The view of `frame` whole, its rows; none where its countries are not complete.
    pub(crate) fn build(frame: &DecodedFrame, refits: bool) -> io::Result<Option<(Box<Self>, Rows)>> {
        let mut owners = Owners::default();
        let mut reasons = Reasons::default();
        let strength = 1 << replay::columns::field(TableId::Ships, "strength")?;
        let Some((state, dirty)) = State::build(frame, refits, strength, &mut owners, &mut reasons)
        else {
            return Ok(None);
        };
        let mut projector = Box::new(Self {
            refits,
            strength,
            owners,
            reasons,
            state,
            published: HashMap::new(),
        });
        let rows = projector.rows(dirty, true);
        Ok(Some((projector, rows)))
    }

    /// The rows the hour of `frame` changed, `touched` what it changed of each table; none
    /// where its countries are not complete.
    pub(crate) fn hour(&mut self, frame: &DecodedFrame, touched: &Touched) -> Option<Rows> {
        let whole = management::TABLES.iter().any(|&table| {
            reads(table, self.refits)
                && (touched.of(table).restated || (NAMING.contains(&table) && touched.of(table).any()))
        });
        if whole {
            let (state, mut dirty) = State::build(
                frame,
                self.refits,
                self.strength,
                &mut self.owners,
                &mut self.reasons,
            )?;
            self.state = state;
            dirty.places.extend(self.published.keys().copied());
            return Some(self.rows(dirty, false));
        }
        let mut dirty = Dirty::default();
        self.state
            .hour(frame, touched, self.strength, &mut self.reasons, &mut dirty);
        Some(self.rows(dirty, false))
    }

    /// The owners the rows name, and whether this is the first hour to carry them.
    pub(crate) fn owners(&mut self) -> (Arc<[String]>, bool) {
        self.owners.shared()
    }

    /// The rows at `dirty`'s places that differ from those last handed on.
    fn rows(&mut self, dirty: Dirty, whole: bool) -> Rows {
        let mut places: Vec<Place> = dirty.places.into_iter().collect();
        places.sort_unstable();
        let mut rows = Vec::new();
        for place in places {
            let now = self.state.row(&place);
            let old = self.published.get(&place);
            if now.as_deref() == old.map(|cells| &**cells) && !whole {
                continue;
            }
            let key = if place.stream == layout::REFUSALS {
                Key::Token(self.reasons.label(place.key as u32).clone())
            } else {
                Key::Number(place.key)
            };
            match &now {
                Some(cells) => {
                    self.published
                        .insert(place, cells.clone().into_boxed_slice());
                }
                None => {
                    self.published.remove(&place);
                    if whole {
                        continue;
                    }
                }
            }
            rows.push(Row {
                owner: place.owner,
                stream: place.stream,
                key,
                cells: now,
            });
        }
        Rows {
            owners: self.owners.names(),
            whole,
            rows,
        }
    }
}

impl State {
    /// The view of `frame` whole, and every place it holds a row at.
    fn build(
        frame: &DecodedFrame,
        refits: bool,
        strength: u64,
        owners: &mut Owners,
        reasons: &mut Reasons,
    ) -> Option<(Self, Dirty)> {
        let identity = Identity::read(frame, owners)?;
        dashboard_base::prepared::prepass::note("naval-yards.project");
        let mut shared = Shared::default();
        let mut refusals = Refusals::default();
        let mut ships = Ships::default();
        let mut dirty = Dirty::default();
        let mut cx = Cx {
            frame,
            countries: &identity.countries,
            identity: &identity,
            shared: &mut shared,
            naming: Naming::new(frame),
            reasons: &mut *reasons,
            refusals: &mut refusals,
            strength,
        };
        let view = if refits {
            View::Refits(Refits::build(&mut cx, &mut ships, &mut dirty))
        } else {
            View::Repairs(Repairs::build(&mut cx, &mut ships, &mut dirty))
        };
        drop(cx);
        let mut state = Self {
            identity,
            shared,
            refusals,
            ships,
            view,
            totals: HashMap::new(),
        };
        dirty.totals_all = true;
        state.totals(frame, strength, reasons, &mut dirty);
        Some((state, dirty))
    }

    /// The hour's changes, `touched`, into the places they reach.
    fn hour(
        &mut self,
        frame: &DecodedFrame,
        touched: &Touched,
        strength: u64,
        reasons: &mut Reasons,
        dirty: &mut Dirty,
    ) {
        let Self {
            identity,
            shared,
            refusals,
            ships,
            view,
            ..
        } = self;
        let mut cx = Cx {
            frame,
            countries: &identity.countries,
            identity: &*identity,
            shared,
            naming: Naming::new(frame),
            reasons: &mut *reasons,
            refusals,
            strength,
        };
        // A definition, or a token's spelling, names only the ships and lines reading it.
        let mut variants: Vec<u64> = keys(touched.of(TableId::VariantDefinitions))
            .map(|key| key as u64)
            .collect();
        cx.shared.redefined(variants.iter().copied());
        let mut tokens: Vec<u32> = keys(touched.of(TableId::LexicalTokens))
            .map(|key| key as u32)
            .collect();
        tokens.sort_unstable();
        variants.extend(cx.shared.relexed(&tokens));
        variants.sort_unstable();
        variants.dedup();
        let mut again = BTreeSet::new();
        for token in &tokens {
            again.extend(ships.classes.get(token).into_iter().flatten());
        }
        for variant in &variants {
            again.extend(ships.variants.get(variant).into_iter().flatten());
        }
        let ship_rows = touched.of(TableId::Ships);
        for &(key, mask) in &ship_rows.written {
            if mask == strength {
                ships.strengthen(frame, dirty, key as u64);
            } else {
                again.insert(key as u64);
            }
        }
        again.extend(ship_rows.deleted.iter().map(|&key| key as u64));
        again.extend(keys(touched.of(TableId::ShipModels)).map(|key| key as u64));
        let mut tasks: BTreeSet<u64> = BTreeSet::new();
        for table in [
            TableId::TaskForceCounterInputs,
            TableId::NavalRepairTaskForces,
            TableId::TaskForces,
        ] {
            tasks.extend(keys(touched.of(table)).map(|key| key as u64));
        }
        tasks.extend(keys(touched.of(TableId::NavalRepairChildren)).map(|key| (key >> 32) as u64));
        for task in tasks {
            again.extend(ships.tasks.get(&task).into_iter().flatten());
        }
        match view {
            View::Repairs(repairs) => repairs.hour(&mut cx, ships, dirty, touched),
            View::Refits(refits) => refits.hour(&mut cx, ships, dirty, touched, &variants),
        }
        for id in again {
            ships.project(&mut cx, dirty, id);
        }
        drop(cx);
        self.totals(frame, strength, reasons, dirty);
    }

    /// The totals of the owners `dirty` names, every current owner's where it names all.
    fn totals(&mut self, frame: &DecodedFrame, strength: u64, reasons: &mut Reasons, dirty: &mut Dirty) {
        let mut owners: BTreeSet<u32> = std::mem::take(&mut dirty.totals).into_iter().collect();
        if dirty.totals_all {
            owners.extend(self.identity.current.iter().copied());
            owners.extend(self.totals.keys().copied());
        }
        let Self {
            identity,
            shared,
            refusals,
            view,
            totals,
            ..
        } = self;
        let mut cx = Cx {
            frame,
            countries: &identity.countries,
            identity: &*identity,
            shared,
            naming: Naming::new(frame),
            reasons: &mut *reasons,
            refusals,
            strength,
        };
        for owner in owners {
            let current = cx.identity.current.binary_search(&owner).is_ok();
            let cells = match view {
                _ if !current => None,
                View::Repairs(repairs) => repairs.totals(&mut cx, dirty, owner),
                View::Refits(refits) => refits.totals(&mut cx, dirty, owner),
            };
            match cells {
                Some(cells) => totals.insert(owner, cells),
                None => totals.remove(&owner),
            };
            dirty.place(owner, layout::TOTALS, 0);
        }
    }

    /// The row at `place` as the projection holds it.
    fn row(&self, place: &Place) -> Option<Vec<Spelled>> {
        match place.stream {
            layout::HOLDERS => match &self.view {
                View::Repairs(repairs) => repairs.holder(place.owner, place.key),
                View::Refits(refits) => refits.holder(place.owner, place.key),
            }
            .map(<[Spelled]>::to_vec),
            layout::SHIPS => self.ships.row(place.owner, place.key),
            layout::TOTALS => self.totals.get(&place.owner).map(|cells| cells.to_vec()),
            _ => {
                let count = self
                    .refusals
                    .counts
                    .get(&(place.owner, place.key as u32))
                    .copied()
                    .unwrap_or(0);
                (count > 0).then(|| vec![Spelled::Integer(i128::from(count))])
            }
        }
    }
}

/// The keys an hour wrote or deleted of a table.
fn keys(touch: &Touch) -> impl Iterator<Item = u128> + '_ {
    touch
        .written
        .iter()
        .map(|&(key, _)| key)
        .chain(touch.deleted.iter().copied())
}

/// `value` taken from `key`'s values in `map`, and `key` with it once it has none.
fn remove<K: Eq + std::hash::Hash, V: PartialEq>(map: &mut HashMap<K, Vec<V>>, key: K, value: V) {
    if let Some(values) = map.get_mut(&key) {
        if let Some(at) = values.iter().position(|held| *held == value) {
            values.swap_remove(at);
        }
        if values.is_empty() {
            map.remove(&key);
        }
    }
}
