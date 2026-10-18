//! Historical names for a stable label column, never for filling a dated label.
use super::*;
use crate::products::war_casualties::same_name_rows;
use installed_content_country_names::{
    country_names, country_names::CountryNames, country_names::Operands,
};
use replay_frame::frame::SharedView;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub(super) struct CountryLabels {
    /// By `country_names::TABLES`: the views of the last call past the
    /// identity test, held by identity so a later change copies none, and the
    /// views names were last read from.
    seen: Vec<ViewIdentity>,
    read: Vec<SharedView>,
    inputs: HashMap<u32, Option<Result<Operands, String>>>,
    resolved: HashMap<u32, String>,
    names: HashMap<u32, BTreeSet<String>>,
    involved: HashSet<u32>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) resolutions: usize,
}

impl CountryLabels {
    pub fn observe(&mut self, frame: &DecodedFrame, cat: Option<&Catalogue>) {
        let Some(cat) = cat else { return };
        let views = country_names::TABLES.map(|table| &frame.tables[table.index()]);
        // As before, views unchanged since that call change nothing, whatever
        // ReservedCountryNaming (read only for slot zero) holds.
        if self.seen.len() == views.len()
            && country_names::TABLES
                .into_iter()
                .zip(&self.seen)
                .zip(views)
                .all(|((table, seen), view)| {
                    table == TableId::ReservedCountryNaming || seen.is(view)
                })
        {
            return;
        }
        self.seen = views.map(SharedView::identity).into();
        // Nor do views agreeing with the last read on every operand it read.
        if self.read.len() == views.len()
            && country_names::TABLES
                .into_iter()
                .zip(&self.read)
                .zip(views)
                .all(|((table, read), view)| same_name_rows(table, read, view))
        {
            return;
        }
        self.read = views.map(Clone::clone).into();
        let Ok(countries) = frame.complete::<tables::Countries>() else {
            return;
        };
        let names = CountryNames::read(frame, &cat.localisation);
        let inputs: HashMap<_, _> = countries
            .iter()
            .map(|country| {
                let slot = country.canonical_slot;
                (slot, names.operands(slot).cloned())
            })
            .collect();
        let mut changed: HashSet<_> = inputs
            .iter()
            .filter(|&(slot, input)| self.inputs.get(slot) != Some(input))
            .map(|(&slot, _)| slot)
            .chain(
                self.inputs
                    .keys()
                    .filter(|slot| !inputs.contains_key(*slot))
                    .copied(),
            )
            .collect();
        // Subject substitutions can resolve an overlord's name or adjective.
        let mut dependents: HashMap<u32, Vec<u32>> = HashMap::new();
        for (&slot, input) in &inputs {
            if let Some(Ok(input)) = input
                && let Some(overlord) = input.overlord
            {
                dependents.entry(overlord).or_default().push(slot);
            }
        }
        let mut pending: Vec<_> = changed.iter().copied().collect();
        let mut index = 0;
        while index < pending.len() {
            for &slot in dependents.get(&pending[index]).into_iter().flatten() {
                if changed.insert(slot) {
                    pending.push(slot);
                }
            }
            index += 1;
        }
        for slot in pending {
            self.resolved.remove(&slot);
            if !inputs.contains_key(&slot) {
                continue;
            }
            #[cfg(any(test, feature = "test-support"))]
            {
                self.resolutions += 1;
            }
            if let Ok(name) = names
                .name(slot)
                .and_then(|name| catalogue::plain_text(&name))
            {
                self.resolved.insert(slot, name);
            }
        }
        self.inputs = inputs;
        for country in countries {
            if let Some(name) = self.resolved.get(&country.canonical_slot) {
                self.names
                    .entry(country.id)
                    .or_default()
                    .insert(name.clone());
            }
        }
    }

    pub fn include(&mut self, inputs: &Hour, observed: &[Observed]) {
        let mut tags: HashSet<&str> = observed
            .iter()
            .flat_map(|o| {
                o.data
                    .viewpoint_side
                    .iter()
                    .chain(&o.data.opposing_side)
                    .map(String::as_str)
            })
            .collect();
        if let Some(tag) = inputs.tags.get(&inputs.view) {
            tags.insert(tag);
        }
        for (&raw, slot) in &inputs.raw_to_slot {
            if inputs
                .tags
                .get(slot)
                .is_some_and(|tag| tags.contains(tag.as_str()))
            {
                self.involved.insert(raw);
            }
        }
    }

    /// The names read so far by raw country id and the countries involved:
    /// all [`Self::finish`] reads, in key order.
    pub fn saved(&self) -> (BTreeMap<u32, BTreeSet<String>>, BTreeSet<u32>) {
        (
            self.names
                .iter()
                .map(|(&raw, names)| (raw, names.clone()))
                .collect(),
            self.involved.iter().copied().collect(),
        )
    }

    /// Labels holding `saved` names and involved countries and no read
    /// operands: observing the frame they were last read from reads them
    /// again and adds no name.
    pub fn restored((names, involved): (BTreeMap<u32, BTreeSet<String>>, BTreeSet<u32>)) -> Self {
        Self {
            names: names.into_iter().collect(),
            involved: involved.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Held operands, names and views, counted for a walk cache's byte estimate.
    pub fn entries(&self) -> usize {
        self.seen.len()
            + self.read.len()
            + self.inputs.len()
            + self.resolved.len()
            + self.names.values().map(BTreeSet::len).sum::<usize>()
            + self.involved.len()
    }

    pub fn finish(self) -> Vec<String> {
        self.names
            .into_iter()
            .filter(|(raw, _)| self.involved.contains(raw))
            .flat_map(|(_, names)| names)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}
