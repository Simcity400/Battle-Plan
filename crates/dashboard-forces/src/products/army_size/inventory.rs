//! Division baselines and template-weighted country composition, kept from the walk's
//! divisions ([`dashboard_world::Divisions`]) and its template columns: an hour reads only the
//! rows it changed, and marks the countries whose totals or composition they move.
use super::{ArmyAttribution, ArmyMeasure, Mix, Totals, channel, owner};
use dashboard_base::prepared::frame_changes::field_mask;
use dashboard_text::metadata::link_slot;
use dashboard_world::{Division, Divisions};
use replay::{columns, segments::State};
use replay_data::rows::{CountryLinkRow, LexicalTokenRow, RowCodec, TableId, TableSchema, tables};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::LazyLock,
};

/// The DivisionTemplateIcons fields a composition reads; the template is the key.
static TEMPLATE_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::DivisionTemplateIcons,
        &["subunits", "model_unreadable"],
    )
});

pub(super) enum Applied<R> {
    Reset(Vec<(Option<R>, Option<R>)>),
    Changes(Vec<(Option<R>, Option<R>)>),
    Same,
    Missing,
}

/// Every live row of `table`, by key.
pub(super) fn rows<R: RowCodec>(state: &State, table: TableId, format: u16) -> io::Result<Vec<R>> {
    let held = columns::table(state, table)?;
    let mut slots: Vec<_> = held
        .live()
        .ones()
        .filter_map(|slot| held.key(slot).map(|key| (key, slot)))
        .collect();
    slots.sort_unstable();
    slots
        .into_iter()
        .map(|(_, slot)| columns::row(state, table, slot, format))
        .collect()
}

/// One table's rows as the walk held them at the hour last applied.
pub(super) struct Sparse<T: TableSchema> {
    pub(super) rows: HashMap<u128, T::Row>,
    pub(super) baseline: Option<i32>,
    error: Option<String>,
}

impl<T: TableSchema> Default for Sparse<T> {
    fn default() -> Self {
        Self {
            rows: HashMap::new(),
            baseline: None,
            error: None,
        }
    }
}

fn check_key_width<R: RowCodec>(key: u128) -> io::Result<()> {
    let bits = R::KEY_BITS;
    if (bits == 0 && key != 0) || (bits < 128 && key >> bits != 0) {
        return Err(io::Error::other("row key exceeds native width"));
    }
    Ok(())
}

impl<T: TableSchema> Sparse<T> {
    /// A whole table's checks: native key widths, valid key fields, and keys unique and
    /// ascending.
    fn validate_rows(rows: &[T::Row]) -> io::Result<()> {
        let mut previous = None;
        for row in rows {
            check_key_width::<T::Row>(row.key())?;
            row.check_key_fields()?;
            if previous.is_some_and(|old| old >= row.key()) {
                return Err(io::Error::other("row keys must be unique and ascending"));
            }
            previous = Some(row.key());
        }
        Ok(())
    }

    /// Refuse the table: no rows are held until it is read whole again.
    fn refuse(&mut self, error: io::Error) -> Applied<T::Row> {
        self.error = Some(error.to_string());
        self.rows.clear();
        self.baseline = None;
        Applied::Missing
    }

    /// Hold `rows` whole, with each row that differs from the held one when
    /// `compare_snapshot`.
    fn snapshot(&mut self, rows: Vec<T::Row>, compare_snapshot: bool) -> Applied<T::Row> {
        let mut edits = Vec::new();
        if compare_snapshot {
            let mut previous = std::mem::take(&mut self.rows);
            for row in rows {
                let old = previous.remove(&row.key());
                if old.as_ref() != Some(&row) {
                    edits.push((old, Some(row.clone())));
                }
                self.rows.insert(row.key(), row);
            }
            edits.extend(previous.into_values().map(|old| (Some(old), None)));
        } else {
            self.rows = rows.into_iter().map(|row| (row.key(), row)).collect();
        }
        Applied::Reset(edits)
    }

    /// Apply the hour's columns to the held rows: whole when `whole`, without held rows, or
    /// when the hour deletes a key not held; otherwise only the rows it changed. A change to
    /// no field in `fields` leaves a held row as it was: nothing reads those fields.
    pub(super) fn apply(
        &mut self,
        state: &State,
        (format, hour): (u16, i32),
        fields: u64,
        compare_snapshot: bool,
        whole: bool,
    ) -> Applied<T::Row> {
        match self.read(state, (format, hour), fields, compare_snapshot, whole) {
            Ok(applied) => applied,
            Err(error) => self.refuse(error),
        }
    }

    fn read(
        &mut self,
        state: &State,
        (format, hour): (u16, i32),
        fields: u64,
        compare_snapshot: bool,
        whole: bool,
    ) -> io::Result<Applied<T::Row>> {
        if !columns::complete(state, T::ID) {
            self.error = None;
            self.rows.clear();
            self.baseline = None;
            return Ok(Applied::Missing);
        }
        let table = columns::table(state, T::ID)?;
        let changes = columns::changes(state, T::ID)?;
        if whole
            || self.baseline.is_none_or(|at| at >= hour)
            || changes
                .deleted_keys
                .iter()
                .any(|key| !self.rows.contains_key(key))
        {
            let rows = rows::<T::Row>(state, T::ID, format)?;
            Self::validate_rows(&rows)?;
            self.error = None;
            self.baseline = Some(hour);
            return Ok(self.snapshot(rows, compare_snapshot));
        }
        let mut slots = BTreeSet::new();
        for (&slot, &mask) in changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        {
            if mask & fields != 0
                || table
                    .key(slot)
                    .is_some_and(|key| !self.rows.contains_key(&key))
            {
                slots.insert(slot);
            }
        }
        slots.extend(changes.inserted.iter().copied());
        let mut read = Vec::with_capacity(slots.len());
        for slot in slots {
            let row: T::Row = columns::row(state, T::ID, slot, format)?;
            row.check_key_fields()?;
            read.push(row);
        }
        let mut edits = Vec::with_capacity(read.len() + changes.deleted_keys.len());
        for key in &changes.deleted_keys {
            edits.push((self.rows.remove(key), None));
        }
        for row in read {
            edits.push((self.rows.insert(row.key(), row.clone()), Some(row)));
        }
        self.error = None;
        self.baseline = Some(hour);
        Ok(if edits.is_empty() {
            Applied::Same
        } else {
            Applied::Changes(edits)
        })
    }
}

#[derive(Default)]
struct Core {
    count: [i128; 2],
    hp: [i128; 2],
    bad_hp: [i128; 2],
    manpower: [i128; 2],
    bad_manpower: [i128; 2],
}

#[derive(Default)]
struct Parts {
    values: [BTreeMap<String, i128>; 2],
    total: [i128; 2],
    missing: [i128; 2],
    version: u64,
}

type Composition = Option<BTreeMap<String, i64>>;

#[derive(Default)]
pub(super) struct Inventory {
    /// Whether the walk's divisions are held, and its manpower, as last applied.
    known: bool,
    land: bool,
    /// The divisions whose aid-relevant owners the hour changed; None when it read them whole.
    pub(super) aid_changes: Option<Vec<u64>>,
    templates: Sparse<tables::DivisionTemplateIcons>,
    core: HashMap<u32, Core>,
    // A template edit touches its country weights, not every division using it.
    members: HashMap<u64, HashMap<u32, [i128; 2]>>,
    compiled: HashMap<u64, Composition>,
    parts: HashMap<u32, Parts>,
    unknown_owner: [i128; 2],
    pub(super) dirty: BTreeSet<u32>,
    names: HashMap<u32, String>,
    token_templates: HashMap<u32, HashSet<u64>>,
    dirty_templates: HashSet<u64>,
    weights: HashMap<(u64, u32), [i128; 2]>,
    rebuild_parts: bool,
    generation: u64,
}

fn edit_core(
    division: &Division,
    manpower: Option<i32>,
    sign: i128,
    links: &[CountryLinkRow],
    core: &mut HashMap<u32, Core>,
    unknown: &mut [i128; 2],
) -> [Option<u32>; 2] {
    let slots = [
        link_slot(links, owner(&division.row())),
        link_slot(links, division.roster_owner),
    ];
    for (index, slot) in slots.iter().enumerate() {
        let Some(slot) = slot else {
            unknown[index] += sign;
            continue;
        };
        let total = core.entry(*slot).or_default();
        total.count[index] += sign;
        edit_manpower(total, index, manpower, sign);
        edit_hp(total, index, division, sign);
    }
    slots
}

/// Combined strength counts current hit points that are finite and not negative; any other
/// value leaves the total unknown.
fn edit_hp(core: &mut Core, index: usize, division: &Division, sign: i128) {
    if dashboard_base::products::finite_fixed64(division.hit_points) && division.hit_points >= 0 {
        core.hp[index] += sign * i128::from(division.hit_points);
    } else {
        core.bad_hp[index] += sign;
    }
}

/// The manpower total sums recorded signed dwords; a division without land inputs leaves it
/// unknown.
fn edit_manpower(core: &mut Core, index: usize, manpower: Option<i32>, sign: i128) {
    match manpower {
        Some(amount) => core.manpower[index] += sign * i128::from(amount),
        None => core.bad_manpower[index] += sign,
    }
}

fn edit_weights(
    template: u64,
    slots: [Option<u32>; 2],
    sign: i128,
    members: &mut HashMap<u64, HashMap<u32, [i128; 2]>>,
    pending: &mut HashMap<(u64, u32), [i128; 2]>,
) {
    for (index, slot) in slots.into_iter().enumerate() {
        let Some(slot) = slot else {
            continue;
        };
        let countries = members.entry(template).or_default();
        let counts = countries.entry(slot).or_default();
        counts[index] += sign;
        if *counts == [0, 0] {
            countries.remove(&slot);
        }
        if countries.is_empty() {
            members.remove(&template);
        }
        pending.entry((template, slot)).or_default()[index] += sign;
    }
}

fn edit_parts(parts: &mut HashMap<u32, Parts>, slot: u32, counts: [i128; 2], value: &Composition) {
    if counts == [0, 0] {
        return;
    }
    let target = parts.entry(slot).or_default();
    target.version = target.version.wrapping_add(1);
    for (index, count) in counts.into_iter().enumerate() {
        if count == 0 {
            continue;
        }
        if let Some(value) = value {
            for (name, amount) in value {
                let delta = count * i128::from(*amount);
                target.total[index] += delta;
                let sum = target.values[index].entry(name.clone()).or_default();
                *sum += delta;
                if *sum == 0 {
                    target.values[index].remove(name);
                }
            }
        } else {
            target.missing[index] += count;
        }
    }
}

impl Inventory {
    /// Apply the hour's divisions and template columns through `links` (None while
    /// CountryLinks is not whole); `changed_links` when the hour changed them. Each country
    /// whose totals or composition they can change is marked dirty.
    pub fn apply(
        &mut self,
        state: &State,
        (format, hour): (u16, i32),
        whole: bool,
        (links, changed_links): (Option<&[CountryLinkRow]>, bool),
        walked: &Divisions,
    ) {
        self.dirty.clear();
        let links = links.unwrap_or_default();
        let templates = self
            .templates
            .apply(state, (format, hour), *TEMPLATE_FIELDS, true, whole);
        match templates {
            Applied::Missing => {
                self.dirty_templates.extend(self.compiled.keys().copied());
                self.token_templates.clear();
            }
            Applied::Reset(edits) | Applied::Changes(edits) => {
                for (old, new) in edits {
                    if let Some(old) = &old {
                        self.dirty_templates.insert(old.template);
                        for unit in &old.subunits {
                            if let Some(templates) = self.token_templates.get_mut(&unit.token) {
                                templates.remove(&old.template);
                                if templates.is_empty() {
                                    self.token_templates.remove(&unit.token);
                                }
                            }
                        }
                    }
                    if let Some(new) = &new {
                        self.dirty_templates.insert(new.template);
                        for unit in &new.subunits {
                            self.token_templates
                                .entry(unit.token)
                                .or_default()
                                .insert(new.template);
                        }
                    }
                }
            }
            Applied::Same => {}
        }
        let (divisions, land) = (walked.divisions(), walked.manpower());
        let manpower = |division: u64| land.holds().then(|| land.get(division).copied()).flatten();
        let edited = divisions.holds() && !divisions.whole();
        self.aid_changes = edited.then(|| {
            divisions
                .edits()
                .iter()
                .filter(|(_, old, new)| {
                    (*old).zip(*new).is_none_or(|(old, new)| {
                        old.logical_owner != new.logical_owner
                            || owner(&old.row()) != owner(&new.row())
                            || old.roster_owner != new.roster_owner
                    })
                })
                .map(|&(id, _, _)| id)
                .collect()
        });
        let rebuild = changed_links
            || !edited
            || land.whole()
            || land.holds() != self.land
            || divisions.holds() != self.known;
        self.known = divisions.holds();
        self.land = land.holds();
        if rebuild {
            self.dirty.extend(self.core.keys().copied());
            self.core.clear();
            self.members.clear();
            self.weights.clear();
            self.unknown_owner = [0, 0];
            self.rebuild_parts = true;
            for (id, division) in divisions.iter() {
                let slots = edit_core(
                    division,
                    manpower(id),
                    1,
                    links,
                    &mut self.core,
                    &mut self.unknown_owner,
                );
                self.dirty.extend(slots.into_iter().flatten());
                edit_weights(
                    division.template,
                    slots,
                    1,
                    &mut self.members,
                    &mut self.weights,
                );
            }
            return;
        }
        // Each division the hour's Divisions or manpower edits name, before and after.
        let mut touched: BTreeMap<u64, [(Option<Division>, Option<i32>); 2]> = BTreeMap::new();
        for &(id, old, new) in divisions.edits() {
            let manpower = manpower(id);
            touched.insert(id, [(old, manpower), (new, manpower)]);
        }
        for &(id, old, new) in land.edits() {
            let current = divisions.get(id).copied();
            let entry = touched
                .entry(id)
                .or_insert([(current, None), (current, None)]);
            entry[0].1 = old;
            entry[1].1 = new;
        }
        for (_, [(old, old_manpower), (new, new_manpower)]) in touched {
            // With the same census owners and template, only HP and manpower affect these
            // totals. Logical-host edits have their own aid invalidation.
            if let (Some(old), Some(new)) = (&old, &new)
                && owner(&old.row()) == owner(&new.row())
                && old.roster_owner == new.roster_owner
                && old.template == new.template
            {
                for (index, raw) in [owner(&new.row()), new.roster_owner]
                    .into_iter()
                    .enumerate()
                {
                    if let Some(slot) = link_slot(links, raw) {
                        let core = self.core.entry(slot).or_default();
                        edit_hp(core, index, old, -1);
                        edit_hp(core, index, new, 1);
                        edit_manpower(core, index, old_manpower, -1);
                        edit_manpower(core, index, new_manpower, 1);
                        self.dirty.insert(slot);
                    }
                }
                continue;
            }
            let before = old.as_ref().map(|division| {
                edit_core(
                    division,
                    old_manpower,
                    -1,
                    links,
                    &mut self.core,
                    &mut self.unknown_owner,
                )
            });
            let after = new.as_ref().map(|division| {
                edit_core(
                    division,
                    new_manpower,
                    1,
                    links,
                    &mut self.core,
                    &mut self.unknown_owner,
                )
            });
            self.dirty
                .extend(before.into_iter().chain(after).flatten().flatten());
            if before == after
                && old.map(|division| division.template) == new.map(|division| division.template)
            {
                continue; // HP and manpower changes do no composition work.
            }
            if let (Some(division), Some(slots)) = (old, before) {
                edit_weights(
                    division.template,
                    slots,
                    -1,
                    &mut self.members,
                    &mut self.weights,
                );
            }
            if let (Some(division), Some(slots)) = (new, after) {
                edit_weights(
                    division.template,
                    slots,
                    1,
                    &mut self.members,
                    &mut self.weights,
                );
            }
        }
    }

    /// Reads the sub-unit names the hour changed, then recomposes what they and the hour's
    /// division edits moved.
    pub(super) fn compose(&mut self, state: &State, format: u16, whole: bool) -> io::Result<()> {
        let table = TableId::LexicalTokens;
        let mut edits = Vec::new();
        if !columns::complete(state, table) {
            edits.extend(self.names.keys().copied().map(|token| (token, None)));
        } else if whole {
            let mut names: HashMap<_, _> = rows::<LexicalTokenRow>(state, table, format)?
                .into_iter()
                .filter_map(|row| {
                    String::from_utf8(row.literal)
                        .ok()
                        .map(|name| (row.token, name))
                })
                .collect();
            for token in self.names.keys() {
                if !names.contains_key(token) {
                    edits.push((*token, None));
                }
            }
            edits.extend(names.drain().map(|(token, name)| (token, Some(name))));
        } else {
            let changes = columns::changes(state, table)?;
            edits.extend(changes.deleted_keys.iter().map(|&key| (key as u32, None)));
            let slots: BTreeSet<_> = changes
                .changed
                .iter()
                .chain(&changes.fill_slots)
                .chain(&changes.inserted)
                .copied()
                .collect();
            for slot in slots {
                let row: LexicalTokenRow = columns::row(state, table, slot, format)?;
                edits.push((row.token, String::from_utf8(row.literal).ok()));
            }
        }
        for (token, name) in edits {
            if self.names.get(&token) != name.as_ref() {
                self.dirty_templates.extend(
                    self.token_templates
                        .get(&token)
                        .into_iter()
                        .flatten()
                        .copied(),
                );
                match name {
                    Some(name) => {
                        self.names.insert(token, name);
                    }
                    None => {
                        self.names.remove(&token);
                    }
                }
            }
        }
        self.compose_parts();
        Ok(())
    }

    fn compose_parts(&mut self) {
        if self.rebuild_parts {
            self.dirty.extend(self.parts.keys().copied());
            self.parts.clear();
            self.rebuild_parts = false;
            self.generation = self.generation.wrapping_add(1);
        }
        let unknown = None;
        for ((template, slot), counts) in self.weights.drain() {
            self.dirty.insert(slot);
            edit_parts(
                &mut self.parts,
                slot,
                counts,
                self.compiled.get(&template).unwrap_or(&unknown),
            );
        }
        for template in self.dirty_templates.drain() {
            let value =
                self.templates
                    .rows
                    .get(&u128::from(template))
                    .and_then(|row| {
                        if row.model_unreadable != 0 {
                            return None;
                        }
                        let mut parts = BTreeMap::<String, i64>::new();
                        for unit in &row.subunits {
                            // Token zero names the null-object definition, not a battalion.
                            if unit.token == 0 {
                                continue;
                            }
                            if unit.count < 0 {
                                return None;
                            }
                            let name = self.names.get(&unit.token).cloned().unwrap_or_else(|| {
                                format!("Unresolved sub-unit token {}", unit.token)
                            });
                            let count = parts.entry(name).or_default();
                            *count = count.checked_add(i64::from(unit.count))?;
                        }
                        Some(parts)
                    });
            let old = self.compiled.get(&template).unwrap_or(&unknown);
            if old != &value {
                for (slot, counts) in self.members.get(&template).into_iter().flatten() {
                    self.dirty.insert(*slot);
                    edit_parts(&mut self.parts, *slot, counts.map(|count| -count), old);
                    edit_parts(&mut self.parts, *slot, *counts, &value);
                }
            }
            if self.templates.rows.contains_key(&u128::from(template)) {
                self.compiled.insert(template, value);
            } else {
                self.compiled.remove(&template);
            }
        }
    }

    /// Whether the divisions were held as last applied.
    pub fn known(&self) -> bool {
        self.known
    }

    /// Each table whose rows could not be read, with why: the walk's divisions and manpower,
    /// and the templates.
    pub fn errors<'a>(&'a self, walked: &'a Divisions) -> impl Iterator<Item = (TableId, &'a str)> {
        [
            (TableId::Divisions, walked.divisions().refusal()),
            (
                TableId::DivisionTemplateIcons,
                self.templates.error.as_deref(),
            ),
            (TableId::DivisionLandInputs, walked.manpower().refusal()),
        ]
        .into_iter()
        .filter_map(|(table, reason)| reason.map(|reason| (table, reason)))
    }

    pub fn composition_known(&self) -> bool {
        self.known() && self.templates.baseline.is_some()
    }

    pub fn unreadable(&self) -> usize {
        self.unknown_owner.iter().sum::<i128>() as usize
    }

    pub fn totals(&self, slot: u32, composition: bool) -> Totals {
        let mut total = [None; 8];
        if !self.known() {
            return total;
        }
        let core = self.core.get(&slot);
        let parts = self.parts.get(&slot);
        for attribution in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            let index = usize::from(attribution == ArmyAttribution::Host);
            if self.unknown_owner[index] != 0 {
                continue;
            }
            let count = channel(ArmyMeasure::Divisions, attribution);
            total[count] = Some(core.map_or(0, |core| core.count[index]));
            if core.is_none_or(|core| core.bad_hp[index] == 0) {
                total[count + 1] = Some(core.map_or(0, |core| core.hp[index]));
            }
            if self.land && core.is_none_or(|core| core.bad_manpower[index] == 0) {
                total[channel(ArmyMeasure::Manpower, attribution)] =
                    Some(core.map_or(0, |core| core.manpower[index]));
            }
            if composition && parts.is_none_or(|parts| parts.missing[index] == 0) {
                total[channel(ArmyMeasure::Battalions, attribution)] =
                    Some(parts.map_or(0, |parts| parts.total[index]));
            }
        }
        total
    }

    /// Names the composition of `slot` held now: equal versions, equal compositions.
    pub fn parts_version(&self, slot: u32) -> (u64, u64, [bool; 2]) {
        let parts = self.parts.get(&slot);
        (
            self.generation,
            parts.map_or(0, |parts| parts.version),
            std::array::from_fn(|index| {
                self.unknown_owner[index] == 0
                    && parts.is_none_or(|parts| parts.missing[index] == 0)
            }),
        )
    }

    pub fn parts_at(&self, slot: u32) -> Mix {
        let parts = self.parts.get(&slot);
        let known = self.parts_version(slot).2;
        std::array::from_fn(|index| {
            if !known[index] {
                return None;
            }
            parts.map_or_else(
                || Some(BTreeMap::new()),
                |parts| {
                    parts.values[index]
                        .iter()
                        .map(|(name, value)| {
                            i64::try_from(*value)
                                .ok()
                                .map(|value| (name.clone(), value))
                        })
                        .collect::<Option<BTreeMap<_, _>>>()
                },
            )
        })
    }
}
