use super::*;
use replay_data::rows::{
    OccupationSettingsRow, Recorded, StateAssignedManpowerRow, StateGarrisonRow,
    StateOccupationLawRow,
};

type Buckets<R> = BTreeMap<StateKey, BTreeMap<u128, R>>;

#[derive(Default)]
struct Groups {
    laws: HashMap<u32, (Option<Vec<u8>>, Option<u32>)>,
    failure: Option<String>,
}

#[derive(Default)]
pub(super) struct Dirty {
    pub(super) states: BTreeSet<u128>,
    pub(super) countries: BTreeSet<u32>,
    pub(super) inventory: BTreeSet<u32>,
    pub(super) all: bool,
    pub(super) inventory_all: bool,
}

#[derive(Default)]
pub(super) struct Joins {
    garrison_rows: Rows<StateGarrisonRow>,
    law_rows: Rows<StateOccupationLawRow>,
    settings_rows: Rows<OccupationSettingsRow>,
    assigned_rows: Rows<StateAssignedManpowerRow>,
    garrisons: Buckets<StateGarrisonRow>,
    inventory_rows: BTreeMap<(u32, u128), StateGarrisonRow>,
    bad_inventory: BTreeMap<u128, StateGarrisonRow>,
    laws: Buckets<StateOccupationLawRow>,
    settings: BTreeMap<u32, BTreeMap<u128, OccupationSettingsRow>>,
    groups: HashMap<u32, Groups>,
    assigned: Buckets<StateAssignedManpowerRow>,
    unresolved_assigned: HashMap<u32, BTreeSet<u128>>,
    failures: BTreeMap<(TableId, u128), (String, String)>,
    /// The failures and the countries with ambiguous settings no read has
    /// refused yet.
    unrefused: BTreeSet<(TableId, u128)>,
    unrefused_ambiguous: BTreeSet<u32>,
    assigned_complete: bool,
    laws_complete: bool,
    garrisons_complete: bool,
    equipment_tokens: HashMap<u32, HashMap<u32, usize>>,
    lexical_status: Option<columns::Status>,
}

fn key(links: &[CountryLinkRow], controller: u32, occupied: u32, state: u32) -> Option<StateKey> {
    Some((
        link_slot(links, controller)?,
        link_slot(links, occupied)?,
        state,
    ))
}

fn put<R: Clone>(rows: &mut Buckets<R>, key: Option<StateKey>, raw: u128, value: Option<&R>) {
    let Some(key) = key else { return };
    match value {
        Some(value) => {
            rows.entry(key).or_default().insert(raw, value.clone());
        }
        None => {
            if let Some(bucket) = rows.get_mut(&key) {
                bucket.remove(&raw);
                if bucket.is_empty() {
                    rows.remove(&key);
                }
            }
        }
    }
}

impl Joins {
    pub(super) fn update(
        &mut self,
        hour: &segments::Hour<'_, World>,
        format: u16,
        names: &Countries,
        reset: bool,
    ) -> io::Result<Dirty> {
        if reset {
            *self = Self::default();
        }
        let mut dirty = Dirty::default();
        let links = &names.links;
        let opens = hour.opens || hour.starts || reset;
        let state = hour.state;
        let old_assigned = self.assigned_complete;
        dirty.all = self.laws_complete != columns::complete(state, TableId::StateOccupationLaws);
        dirty.inventory_all =
            self.garrisons_complete != columns::complete(state, TableId::StateGarrisons);
        self.assigned_complete = columns::complete(state, TableId::StateAssignedManpower);
        self.laws_complete = columns::complete(state, TableId::StateOccupationLaws);
        self.garrisons_complete = columns::complete(state, TableId::StateGarrisons);
        let changed =
            self.garrison_rows
                .update(state, TableId::StateGarrisons, opens, u64::MAX, |slot| {
                    columns::row(state, TableId::StateGarrisons, slot, format)
                })?;
        for raw in changed {
            self.forget(TableId::StateGarrisons, raw);
            if let Some(old) = self.garrison_rows.previous.get(&raw).cloned() {
                self.tokens(&old, false);
                dirty.states.insert(u128::from(old.state));
                if let Some(controller) = link_slot(links, old.controller) {
                    dirty.inventory.insert(controller);
                }
                put(
                    &mut self.garrisons,
                    key(links, old.controller, old.occupied, old.state),
                    raw,
                    None,
                );
                if let Some(controller) = link_slot(links, old.controller) {
                    self.inventory_rows.remove(&(controller, raw));
                }
                self.bad_inventory.remove(&raw);
            }
            if let Some(row) = self.garrison_rows.rows.get(&raw).cloned() {
                self.tokens(&row, true);
                dirty.states.insert(u128::from(row.state));
                if let Some(controller) = link_slot(links, row.controller) {
                    dirty.inventory.insert(controller);
                }
                if let Some(controller) = link_slot(links, row.controller) {
                    self.inventory_rows.insert((controller, raw), row.clone());
                }
                if link_slot(links, row.controller)
                    .is_none_or(|slot| !names.index.identities.contains_key(&slot))
                {
                    self.bad_inventory.insert(raw, row.clone());
                }
                let joined = key(links, row.controller, row.occupied, row.state);
                if joined.is_none() {
                    self.fail(
                        TableId::StateGarrisons,
                        raw,
                        "StateGarrisons country link is unresolved",
                        format!(
                            "state {}, controller {}, occupied {}",
                            row.state, row.controller, row.occupied
                        ),
                    );
                }
                put(&mut self.garrisons, joined, raw, Some(&row));
            }
        }
        let changed = self.law_rows.update(
            state,
            TableId::StateOccupationLaws,
            opens,
            u64::MAX,
            |slot| columns::row(state, TableId::StateOccupationLaws, slot, format),
        )?;
        for raw in changed {
            self.forget(TableId::StateOccupationLaws, raw);
            if let Some(old) = self.law_rows.previous.get(&raw) {
                dirty.states.insert(u128::from(old.state));
                put(
                    &mut self.laws,
                    key(links, old.controller, old.occupied, old.state),
                    raw,
                    None,
                );
            }
            if let Some(row) = self.law_rows.rows.get(&raw).cloned() {
                dirty.states.insert(u128::from(row.state));
                let joined = key(links, row.controller, row.occupied, row.state);
                if joined.is_none() {
                    self.fail(
                        TableId::StateOccupationLaws,
                        raw,
                        "StateOccupationLaws country link is unresolved",
                        format!(
                            "state {}, controller {}, occupied {}",
                            row.state, row.controller, row.occupied
                        ),
                    );
                }
                put(&mut self.laws, joined, raw, Some(&row));
            }
        }
        let changed = self.assigned_rows.update(
            state,
            TableId::StateAssignedManpower,
            opens,
            u64::MAX,
            |slot| columns::row(state, TableId::StateAssignedManpower, slot, format),
        )?;
        for raw in changed {
            self.forget(TableId::StateAssignedManpower, raw);
            if let Some(old) = self.assigned_rows.previous.get(&raw) {
                dirty.states.insert(u128::from(old.state));
                if let Some(unresolved) = self.unresolved_assigned.get_mut(&old.state) {
                    unresolved.remove(&raw);
                    if unresolved.is_empty() {
                        self.unresolved_assigned.remove(&old.state);
                    }
                }
                put(
                    &mut self.assigned,
                    key(links, old.controller, old.occupied, old.state),
                    raw,
                    None,
                );
            }
            if let Some(row) = self.assigned_rows.rows.get(&raw).cloned() {
                dirty.states.insert(u128::from(row.state));
                let joined = key(links, row.controller, row.occupied, row.state);
                if joined.is_none() {
                    self.unresolved_assigned
                        .entry(row.state)
                        .or_default()
                        .insert(raw);
                    self.fail(
                        TableId::StateAssignedManpower,
                        raw,
                        "StateAssignedManpower country link is unresolved",
                        format!(
                            "state {}, controller {}, occupied {}",
                            row.state, row.controller, row.occupied
                        ),
                    );
                }
                put(&mut self.assigned, joined, raw, Some(&row));
            }
        }
        if old_assigned != self.assigned_complete {
            dirty.states.extend(
                self.garrison_rows
                    .rows
                    .values()
                    .map(|row| u128::from(row.state)),
            );
        }
        let changed = self.settings_rows.update(
            state,
            TableId::OccupationSettings,
            opens,
            u64::MAX,
            |slot| columns::row(state, TableId::OccupationSettings, slot, format),
        )?;
        for raw in changed {
            self.forget(TableId::OccupationSettings, raw);
            if let Some(old) = self.settings_rows.previous.get(&raw)
                && let Some(slot) = link_slot(links, old.country)
            {
                dirty.countries.insert(slot);
                if let Some(rows) = self.settings.get_mut(&slot) {
                    rows.remove(&raw);
                    if rows.is_empty() {
                        self.settings.remove(&slot);
                    }
                }
            }
            if let Some(row) = self.settings_rows.rows.get(&raw).cloned() {
                if let Some(slot) = link_slot(links, row.country) {
                    dirty.countries.insert(slot);
                    self.settings.entry(slot).or_default().insert(raw, row);
                } else {
                    self.fail(
                        TableId::OccupationSettings,
                        raw,
                        "OccupationSettings country link is unresolved",
                        format!("raw link {}", row.country),
                    );
                }
            }
        }
        for &country in &dirty.countries {
            self.groups(country, links);
        }
        let lexical = columns::changes(state, TableId::LexicalTokens)?;
        let lexical_status = columns::status(state, TableId::LexicalTokens);
        if self.lexical_status != Some(lexical_status) {
            dirty.states.extend(
                self.equipment_tokens
                    .values()
                    .flat_map(|states| states.keys().map(|&state| u128::from(state))),
            );
        }
        self.lexical_status = Some(lexical_status);
        let table = columns::table(state, TableId::LexicalTokens)?;
        for token in lexical
            .changed
            .iter()
            .chain(&lexical.fill_slots)
            .chain(&lexical.inserted)
            .filter_map(|&slot| table.key(slot))
            .chain(lexical.deleted_keys.iter().copied())
        {
            if let Some(states) = self.equipment_tokens.get(&(token as u32)) {
                dirty
                    .states
                    .extend(states.keys().map(|&state| u128::from(state)));
            }
        }
        Ok(dirty)
    }

    fn fail(&mut self, table: TableId, raw: u128, reason: &str, identity: String) {
        self.failures
            .insert((table, raw), (reason.to_owned(), identity));
        self.unrefused.insert((table, raw));
    }

    fn forget(&mut self, table: TableId, raw: u128) {
        self.failures.remove(&(table, raw));
        self.unrefused.remove(&(table, raw));
    }

    fn tokens(&mut self, row: &StateGarrisonRow, add: bool) {
        let Recorded::Value(inputs) = &row.equipment else {
            return;
        };
        for token in inputs
            .rows
            .iter()
            .flat_map(|entry| [entry.equipment_token, entry.archetype_token])
        {
            if add {
                *self
                    .equipment_tokens
                    .entry(token)
                    .or_default()
                    .entry(row.state)
                    .or_default() += 1;
            } else if let Some(states) = self.equipment_tokens.get_mut(&token) {
                if let Some(count) = states.get_mut(&row.state) {
                    *count -= 1;
                    if *count == 0 {
                        states.remove(&row.state);
                    }
                }
                if states.is_empty() {
                    self.equipment_tokens.remove(&token);
                }
            }
        }
    }

    fn groups(&mut self, country: u32, links: &[CountryLinkRow]) {
        self.groups.remove(&country);
        self.unrefused_ambiguous.remove(&country);
        let Some(rows) = self.settings.get(&country) else {
            return;
        };
        let mut groups = Groups::default();
        if rows.len() != 1 {
            self.unrefused_ambiguous.insert(country);
            groups.failure = Some("OccupationSettings canonical country link is ambiguous".into());
        } else {
            let row = rows.first_key_value().expect("settings row").1;
            match &row.group_laws {
                Recorded::NotRecorded => groups.failure = Some(GROUP_LAW_MISSING.into()),
                Recorded::Value(None) => {
                    groups.failure =
                        Some("The occupied-country law inventory capture failed".into())
                }
                Recorded::Value(Some(rows)) => {
                    for row in rows {
                        let Some(occupied) = link_slot(links, row.occupied) else {
                            groups.failure = Some(
                                "The occupied-country law inventory has an unresolved country link"
                                    .into(),
                            );
                            continue;
                        };
                        if groups
                            .laws
                            .insert(occupied, (row.law.clone(), row.scope_country))
                            .is_some()
                        {
                            groups.failure = Some("The occupied-country law inventory has duplicate canonical country links".into());
                        }
                    }
                }
            }
        }
        self.groups.insert(country, groups);
    }

    /// Refuses the read's failed joins the projection has not refused yet: a
    /// refusal is noted once however many reads meet it.
    pub(super) fn refuse(
        &mut self,
        frame: HourFrame<'_>,
        refused: &mut Refusals,
    ) -> io::Result<()> {
        // Their statuses alone: a table not recorded fails the read.
        recorded_rows_in::<tables::StateGarrisons>(frame, 0)?;
        recorded_rows_in::<tables::StateOccupationLaws>(frame, 0)?;
        recorded_rows_in::<tables::OccupationSettings>(frame, 0)?;
        recorded_rows_in::<tables::StateAssignedManpower>(frame, 0)?;
        for key in std::mem::take(&mut self.unrefused) {
            let (reason, identity) = &self.failures[&key];
            refused.note(reason, identity);
        }
        for country in std::mem::take(&mut self.unrefused_ambiguous) {
            refused.note(
                "OccupationSettings canonical country link is ambiguous",
                format!("slot {country}"),
            );
        }
        Ok(())
    }

    pub(super) fn inventories(&self) -> BTreeSet<u32> {
        self.inventory_rows
            .keys()
            .map(|&(country, _)| country)
            .collect()
    }

    pub(super) fn inventory(
        &self,
        country: u32,
        names: &Countries,
        refused: &mut Refusals,
    ) -> (HashSet<u32>, BTreeSet<Identity>) {
        let mut held = HashSet::new();
        let mut occupied = BTreeSet::new();
        for (_, row) in self
            .inventory_rows
            .range((country, 0)..=(country, u128::MAX))
        {
            held.insert(row.state);
            let identity = link_slot(&names.links, row.occupied)
                .and_then(|slot| names.index.identities.get(&slot).map(|&id| (slot, id)));
            if let Some(identity) = identity {
                occupied.insert(identity);
            } else {
                refused.note(
                    "held occupied-country link is unresolved",
                    format!("state {}, raw link {}", row.state, row.occupied),
                );
            }
        }
        (held, occupied)
    }

    pub(super) fn inventory_complete(&self, refused: &mut Refusals) -> bool {
        let mut complete = self.garrisons_complete;
        for row in self.bad_inventory.values() {
            complete = false;
            refused.note(
                "held controller link is unresolved",
                format!("state {}, raw link {}", row.state, row.controller),
            );
        }
        complete
    }
}

impl ReadInputs for Joins {
    fn garrison<'a>(&'a self, _: HourFrame<'a>, key: StateKey) -> Option<&'a StateGarrisonRow> {
        self.garrisons
            .get(&key)?
            .last_key_value()
            .map(|(_, row)| row)
    }
    fn law<'a>(&'a self, _: HourFrame<'a>, key: StateKey) -> Option<&'a StateOccupationLawRow> {
        self.laws.get(&key)?.last_key_value().map(|(_, row)| row)
    }
    fn settings<'a>(
        &'a self,
        _: HourFrame<'a>,
        country: u32,
    ) -> Option<&'a OccupationSettingsRow> {
        let rows = self.settings.get(&country)?;
        (rows.len() == 1).then(|| rows.first_key_value().expect("settings row").1)
    }
    fn group_failure(&self, country: u32) -> Option<&String> {
        self.groups.get(&country)?.failure.as_ref()
    }
    fn group(&self, controller: u32, occupied: u32) -> Option<(Option<&[u8]>, Option<u32>)> {
        self.groups
            .get(&controller)?
            .laws
            .get(&occupied)
            .map(|group| (group.0.as_deref(), group.1))
    }
    fn assigned(&self, key: StateKey) -> i32 {
        self.assigned
            .get(&key)
            .into_iter()
            .flat_map(|rows| rows.values())
            .fold(0_i32, |total, row| total.wrapping_add(row.amount))
    }
    fn assigned_unresolved(&self, state: u32) -> bool {
        self.unresolved_assigned.contains_key(&state)
    }
    fn assigned_complete(&self) -> bool {
        self.assigned_complete
    }
    fn laws_complete(&self) -> bool {
        self.laws_complete
    }
}
