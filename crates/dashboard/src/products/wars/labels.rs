//! Historical names for a stable label column, never for filling a dated label.
use super::*;
use installed_content::country_names::{CountryNames, Operands};

#[derive(Default)]
pub(super) struct CountryLabels {
    tables: Vec<SharedView>,
    inputs: HashMap<u32, Option<Result<Operands, String>>>,
    resolved: HashMap<u32, String>,
    names: HashMap<u32, BTreeSet<String>>,
    involved: HashSet<u32>,
    #[cfg(test)]
    pub(super) resolutions: usize,
}

impl CountryLabels {
    pub fn observe(&mut self, frame: &DecodedFrame, cat: Option<&Catalogue>) {
        let Some(cat) = cat else { return };
        let tables = [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryAppearance,
            TableId::Politics,
            TableId::PoliticalParties,
            TableId::Autonomy,
        ]
        .map(|table| frame.tables[table.index()].clone());
        if self.tables.len() == tables.len()
            && self
                .tables
                .iter()
                .zip(&tables)
                .all(|(a, b)| SharedView::ptr_eq(a, b))
        {
            return;
        }
        self.tables = tables.into();
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
            #[cfg(test)]
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
