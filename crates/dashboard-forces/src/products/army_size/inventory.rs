//! Sparse division baselines and template-weighted country composition.
//! Keys/order are checked before sparse updates; native delta fields use Replay.
use super::*;
use replay::{
    frame::{Frame, ViewIdentity},
    rows::{RowCodec, TableSchema},
};
use std::collections::HashSet;

enum Applied<R> {
    Reset(Vec<(Option<R>, Option<R>)>),
    Changes(Vec<(Option<R>, Option<R>)>),
    Same,
    Missing,
}

struct Sparse<T: TableSchema> {
    rows: HashMap<u128, T::Row>,
    baseline: Option<i32>,
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

impl<T: TableSchema> Sparse<T> {
    // Replay's validate_rows/validate_delta are private. Match their checks
    // using this schema's native key width and row field validation.
    fn validate(update: &TableUpdate) -> io::Result<()> {
        let key = |key: u128| -> io::Result<()> {
            let bits = T::Row::KEY_BITS;
            if (bits == 0 && key != 0) || (bits < 128 && key >> bits != 0) {
                return Err(io::Error::other("row key exceeds native width"));
            }
            Ok(())
        };
        match update {
            TableUpdate::Snapshot(rows) => {
                let rows = rows
                    .rows::<T>()
                    .ok_or_else(|| io::Error::other("snapshot rows have the wrong table type"))?;
                let mut previous = None;
                for row in rows {
                    key(row.key())?;
                    row.check_key_fields()?;
                    if previous.is_some_and(|old| old >= row.key()) {
                        return Err(io::Error::other("row keys must be unique and ascending"));
                    }
                    previous = Some(row.key());
                }
            }
            TableUpdate::Delta {
                changed, deleted, ..
            } => {
                for value in changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                {
                    key(value)?;
                }
                if changed.windows(2).any(|pair| pair[0].key >= pair[1].key)
                    || deleted.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(io::Error::other("delta keys must be unique and ascending"));
                }
                let mut deleted = deleted.iter().peekable();
                for row in changed {
                    while deleted.peek().is_some_and(|value| **value < row.key) {
                        deleted.next();
                    }
                    if deleted.peek().is_some_and(|value| **value == row.key) {
                        return Err(io::Error::other("delta both changes and deletes a key"));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn update(&mut self, frame: &Frame, compare_snapshot: bool) -> Applied<T::Row> {
        self.error = None;
        if let Err(error) = Self::validate(&frame.tables[T::ID.index()]) {
            self.error = Some(error.to_string());
            self.rows.clear();
            self.baseline = None;
            return Applied::Missing;
        }
        let outcome = match &frame.tables[T::ID.index()] {
            TableUpdate::Snapshot(rows) => rows.rows::<T>().map(|rows| {
                let mut edits = Vec::new();
                if compare_snapshot {
                    let mut previous = std::mem::take(&mut self.rows);
                    for row in rows {
                        let old = previous.remove(&row.key());
                        if old.as_ref() != Some(row) {
                            edits.push((old, Some(row.clone())));
                        }
                        self.rows.insert(row.key(), row.clone());
                    }
                    edits.extend(previous.into_values().map(|old| (Some(old), None)));
                } else {
                    self.rows = rows.iter().map(|row| (row.key(), row.clone())).collect();
                }
                Applied::Reset(edits)
            }),
            TableUpdate::Delta {
                baseline,
                changed,
                deleted,
            } if self.baseline == Some(*baseline) && *baseline < frame.hour => {
                let mut edits = Vec::with_capacity(changed.len() + deleted.len());
                let mut valid = true;
                for key in deleted {
                    if let Some(old) = self.rows.remove(key) {
                        edits.push((Some(old), None));
                    } else {
                        self.error = Some("delta deletes an absent key".into());
                        valid = false;
                        break;
                    }
                }
                if valid {
                    for delta in changed {
                        let old = self.rows.remove(&delta.key);
                        match delta.apply::<T::Row>(old.clone()) {
                            Ok(row) if row.check_key_fields().is_ok() => {
                                self.rows.insert(delta.key, row.clone());
                                edits.push((old, Some(row)));
                            }
                            Ok(_) => {
                                self.error = Some("delta row has invalid key fields".into());
                                valid = false;
                                break;
                            }
                            Err(error) => {
                                self.error = Some(error.to_string());
                                valid = false;
                                break;
                            }
                        }
                    }
                }
                valid.then_some(Applied::Changes(edits))
            }
            TableUpdate::Unchanged { baseline }
                if self.baseline == Some(*baseline) && *baseline < frame.hour =>
            {
                Some(Applied::Same)
            }
            _ => None,
        };
        if let Some(outcome) = outcome {
            self.baseline = Some(frame.hour);
            outcome
        } else {
            self.rows.clear();
            self.baseline = None;
            if matches!(
                &frame.tables[T::ID.index()],
                TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. }
            ) {
                self.error
                    .get_or_insert_with(|| "missing table baseline".into());
            }
            Applied::Missing
        }
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
    divisions: Sparse<tables::Divisions>,
    land: Sparse<tables::DivisionLandInputs>,
    templates: Sparse<tables::DivisionTemplateIcons>,
    core: HashMap<u32, Core>,
    // A template edit touches its country weights, not every division using it.
    members: HashMap<u64, HashMap<u32, [i128; 2]>>,
    compiled: HashMap<u64, Composition>,
    parts: HashMap<u32, Parts>,
    unknown_owner: [i128; 2],
    pub(super) dirty: std::collections::BTreeSet<u32>,
    links: Option<ViewIdentity>,
    lexical: Option<ViewIdentity>,
    names: HashMap<u32, String>,
    token_templates: HashMap<u32, HashSet<u64>>,
    dirty_templates: HashSet<u64>,
    weights: HashMap<(u64, u32), [i128; 2]>,
    rebuild_parts: bool,
    generation: u64,
    #[cfg(test)]
    pub applied_divisions: usize,
    #[cfg(test)]
    pub compiled_templates: usize,
}

fn edit_core(
    row: &DivisionRow,
    land: Option<&replay::rows::DivisionLandInputsRow>,
    sign: i128,
    links: &[CountryLinkRow],
    core: &mut HashMap<u32, Core>,
    unknown: &mut [i128; 2],
) -> [Option<u32>; 2] {
    let slots = [
        link_slot(links, owner(row)),
        link_slot(links, row.roster_owner),
    ];
    for (index, slot) in slots.iter().enumerate() {
        let Some(slot) = slot else {
            unknown[index] += sign;
            continue;
        };
        let total = core.entry(*slot).or_default();
        total.count[index] += sign;
        edit_manpower(total, index, land, sign);
        if dashboard_base::products::finite_fixed64(row.hit_points) && row.hit_points >= 0 {
            total.hp[index] += sign * i128::from(row.hit_points);
        } else {
            total.bad_hp[index] += sign;
        }
    }
    slots
}

fn edit_manpower(
    core: &mut Core,
    index: usize,
    land: Option<&replay::rows::DivisionLandInputsRow>,
    sign: i128,
) {
    if let Some(land) = land {
        // The manpower total sums recorded signed dwords.
        let amount = land
            .manpower
            .iter()
            .fold(0i32, |sum, part| sum.wrapping_add(part.amount));
        core.manpower[index] += sign * i128::from(amount);
    } else {
        core.bad_manpower[index] += sign;
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
    pub fn apply_changes(&mut self, frame: &Frame, decoded: &replay::frame::DecodedFrame) {
        self.dirty.clear();
        let links = decoded.complete::<tables::CountryLinks>().ok();
        let view = links.map(|_| &decoded.tables[TableId::CountryLinks.index()]);
        let changed_links = match (&self.links, view) {
            (Some(previous), Some(view)) => !previous.is(view),
            (None, None) => false,
            _ => true,
        };
        self.links = view.map(|view| view.identity());
        let links = links.unwrap_or_default();
        let templates = self.templates.update(frame, true);
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
        let divisions = self.divisions.update(frame, false);
        #[cfg(test)]
        {
            self.applied_divisions += match &divisions {
                Applied::Reset(_) => self.divisions.rows.len(),
                Applied::Changes(edits) => edits.len(),
                _ => 0,
            };
        }
        if changed_links || matches!(&divisions, Applied::Reset(_) | Applied::Missing) {
            self.dirty.extend(self.core.keys().copied());
            self.core.clear();
            self.members.clear();
            self.weights.clear();
            self.unknown_owner = [0, 0];
            self.rebuild_parts = true;
            for row in self.divisions.rows.values() {
                let slots = edit_core(
                    row,
                    self.land.rows.get(&row.key()),
                    1,
                    links,
                    &mut self.core,
                    &mut self.unknown_owner,
                );
                self.dirty.extend(slots.into_iter().flatten());
                edit_weights(row.template, slots, 1, &mut self.members, &mut self.weights);
            }
        } else if let Applied::Changes(edits) = divisions {
            for (old, new) in edits {
                let before = old.as_ref().map(|row| {
                    edit_core(
                        row,
                        self.land.rows.get(&row.key()),
                        -1,
                        links,
                        &mut self.core,
                        &mut self.unknown_owner,
                    )
                });
                let after = new.as_ref().map(|row| {
                    edit_core(
                        row,
                        self.land.rows.get(&row.key()),
                        1,
                        links,
                        &mut self.core,
                        &mut self.unknown_owner,
                    )
                });
                self.dirty
                    .extend(before.into_iter().chain(after).flatten().flatten());
                if before == after
                    && old.as_ref().map(|row| row.template) == new.as_ref().map(|row| row.template)
                {
                    continue; // HP-only changes do no composition work.
                }
                if let (Some(row), Some(slots)) = (old, before) {
                    edit_weights(
                        row.template,
                        slots,
                        -1,
                        &mut self.members,
                        &mut self.weights,
                    );
                }
                if let (Some(row), Some(slots)) = (new, after) {
                    edit_weights(row.template, slots, 1, &mut self.members, &mut self.weights);
                }
            }
        }
        match self.land.update(frame, true) {
            Applied::Changes(edits) | Applied::Reset(edits) => {
                for (old, new) in edits {
                    let id = old
                        .as_ref()
                        .or(new.as_ref())
                        .expect("changed land row")
                        .division;
                    let Some(division) = self.divisions.rows.get(&u128::from(id)) else {
                        continue;
                    };
                    for (index, raw) in [owner(division), division.roster_owner]
                        .into_iter()
                        .enumerate()
                    {
                        let Some(slot) = link_slot(links, raw) else {
                            continue;
                        };
                        self.dirty.insert(slot);
                        let core = self.core.entry(slot).or_default();
                        edit_manpower(core, index, old.as_ref(), -1);
                        edit_manpower(core, index, new.as_ref(), 1);
                    }
                }
            }
            Applied::Missing => {
                self.dirty.extend(self.core.keys().copied());
                for core in self.core.values_mut() {
                    core.manpower = [0, 0];
                    core.bad_manpower = core.count;
                }
            }
            Applied::Same => {}
        }
    }

    pub fn compose(&mut self, decoded: &replay::frame::DecodedFrame) {
        let lexical = decoded.complete::<tables::LexicalTokens>().ok();
        let view = lexical.map(|_| &decoded.tables[TableId::LexicalTokens.index()]);
        let changed = match (&self.lexical, view) {
            (Some(previous), Some(view)) => !previous.is(view),
            (None, None) => false,
            _ => true,
        };
        if changed {
            let names: HashMap<_, _> = lexical
                .unwrap_or_default()
                .iter()
                .filter_map(|row| {
                    std::str::from_utf8(&row.literal)
                        .ok()
                        .map(|name| (row.token, name.to_owned()))
                })
                .collect();
            for token in self.names.keys().chain(names.keys()) {
                if self.names.get(token) != names.get(token) {
                    self.dirty_templates.extend(
                        self.token_templates
                            .get(token)
                            .into_iter()
                            .flatten()
                            .copied(),
                    );
                }
            }
            self.names = names;
            self.lexical = view.map(|view| view.identity());
        }
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
                            // TemplateSubunit: replay/extended_rows/movement.rs:585.
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
            #[cfg(test)]
            {
                self.compiled_templates += 1;
            }
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

    pub fn known(&self) -> bool {
        self.divisions.baseline.is_some()
    }
    pub fn errors(&self) -> impl Iterator<Item = (TableId, &str)> {
        [
            (TableId::Divisions, self.divisions.error.as_deref()),
            (
                TableId::DivisionTemplateIcons,
                self.templates.error.as_deref(),
            ),
            (TableId::DivisionLandInputs, self.land.error.as_deref()),
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
        let mut total = Totals::default();
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
            total.values[count] = Some(core.map_or(0, |core| core.count[index]));
            if core.is_none_or(|core| core.bad_hp[index] == 0) {
                total.values[count + 1] = Some(core.map_or(0, |core| core.hp[index]));
            }
            if self.land.baseline.is_some() && core.is_none_or(|core| core.bad_manpower[index] == 0)
            {
                total.values[channel(ArmyMeasure::Manpower, attribution)] =
                    Some(core.map_or(0, |core| core.manpower[index]));
            }
            if composition && parts.is_none_or(|parts| parts.missing[index] == 0) {
                total.values[channel(ArmyMeasure::Battalions, attribution)] =
                    Some(parts.map_or(0, |parts| parts.total[index]));
            }
        }
        total
    }

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

    pub fn parts_at(&self, slot: u32) -> [Option<BTreeMap<String, i64>>; 2] {
        let parts = self.parts.get(&slot);
        std::array::from_fn(|index| {
            if !self.parts_version(slot).2[index] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::FrameEncoder;

    #[test]
    fn army_size_sparse_rejects_invalid_snapshot_and_delta_keys() {
        let mut encoder = FrameEncoder::new();
        let initial = encoder
            .encode(super::super::test_fixtures::frame(100), false)
            .unwrap();
        let mut observation = super::super::test_fixtures::frame(101);
        if let replay::frame::TableView::Complete(rows) =
            &mut *observation.tables[TableId::Divisions.index()]
        {
            // A whole display step (replay::display), so the row changes.
            rows.rows_mut::<tables::Divisions>().unwrap()[0].hit_points += 100_000;
        }
        let changed = encoder.encode(observation, false).unwrap();
        for case in 0..7 {
            let mut invalid = if case <= 1 {
                initial.clone()
            } else {
                changed.clone()
            };
            invalid.hour = 101;
            match &mut invalid.tables[TableId::Divisions.index()] {
                TableUpdate::Snapshot(rows) => {
                    let rows = rows.rows_mut::<tables::Divisions>().unwrap();
                    if case == 0 {
                        rows.push(rows[0].clone());
                    } else {
                        rows.swap(0, 1);
                    }
                }
                TableUpdate::Delta {
                    changed, deleted, ..
                } => match case {
                    2 => changed.push(changed[0].clone()),
                    3 => changed[0].key = 1u128 << 64,
                    4 => *deleted = vec![2, 2],
                    5 => *deleted = vec![changed[0].key],
                    6 => *deleted = vec![1u128 << 64],
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }
            let mut sparse = Sparse::<tables::Divisions>::default();
            sparse.update(&initial, false);
            assert!(matches!(sparse.update(&invalid, false), Applied::Missing));
            assert!(sparse.rows.is_empty());
            assert!(sparse.baseline.is_none());
            assert!(sparse.error.is_some(), "case {case} must name its refusal");
        }
        let mut invalid = initial.clone();
        invalid.hour = 101;
        if let TableUpdate::Snapshot(rows) =
            &mut invalid.tables[TableId::DivisionTemplateIcons.index()]
        {
            let rows = rows.rows_mut::<tables::DivisionTemplateIcons>().unwrap();
            rows.push(rows[0].clone());
        }
        let mut sparse = Sparse::<tables::DivisionTemplateIcons>::default();
        sparse.update(&initial, true);
        assert!(matches!(sparse.update(&invalid, true), Applied::Missing));
        assert_eq!(
            sparse.error.as_deref(),
            Some("row keys must be unique and ascending")
        );
    }
}
