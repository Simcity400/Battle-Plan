
use std::{collections::BTreeMap, io, slice};

use content_reader_parser::parser::{self, Document, Item, Value};

use super::{GameDate, Save, block, field, invalid, required_block, required_bytes};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActualCountry {
    Reserved,
    Canonical(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryEmissionIdentity {
    pub physical_slot: u32,
    pub own_id: u32,
    pub actual: ActualCountry,
    pub tag: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedCountryEmission {
    pub identity: CountryEmissionIdentity,
    pub intel_present: bool,
}

#[derive(Debug)]
pub struct SaveDocument {
    document: Document,
    date: GameDate,
    campaign_id: Vec<u8>,
    version: Vec<u8>,
}

impl SaveDocument {
    pub fn read(input: &[u8]) -> io::Result<Self> {
        let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
        if input.starts_with(b"HOI4bin") {
            return Err(invalid(
                "binary the game saves cannot be compared; set save_as_binary=no in settings.txt, then create a fresh save",
            ));
        }
        if !input.starts_with(b"HOI4txt") || !input.get(7).is_some_and(u8::is_ascii_whitespace) {
            return Err(invalid(
                "expected a HOI4txt save header; set save_as_binary=no in settings.txt and create a fresh save",
            ));
        }
        let document = parser::parse_save(input)?;
        let date = GameDate::parse(&required_bytes(&document.items, b"date")?)?;
        let campaign_id = required_bytes(&document.items, b"game_unique_id")?;
        let version = required_bytes(&document.items, b"version")?;
        Ok(Self {
            document,
            date,
            campaign_id,
            version,
        })
    }

    pub fn date(&self) -> GameDate {
        self.date
    }

    pub fn campaign_id(&self) -> &[u8] {
        &self.campaign_id
    }

    pub fn version(&self) -> &[u8] {
        &self.version
    }

    pub fn items(&self) -> &[Item] {
        &self.document.items
    }

    pub fn original_country_items(&self) -> io::Result<&[Item]> {
        required_block(&self.document.items, b"countries")
    }

    pub fn representative_country_items(
        &self,
        layout: &[CountryEmissionIdentity],
    ) -> io::Result<Vec<&Item>> {
        let projection = self.project(layout)?;
        let items = self.original_country_items()?;
        Ok(projection
            .representatives
            .iter()
            .map(|&ordinal| &items[ordinal])
            .collect())
    }

    pub fn parse(&self) -> io::Result<Save> {
        super::parse_items(
            &self.document.items,
            CountriesView::from_root(&self.document.items)?,
        )
    }

    pub fn mission_raiding_grid(
        &self,
        keys: [&[u8]; 2],
    ) -> io::Result<super::naval_missions::RaidingGridSave> {
        if self.has_repeated_country_tags()? {
            return Err(invalid(
                "repeated country tags require authenticated physical projection for raiding grid",
            ));
        }
        super::naval_missions::raiding_grid(CountriesView::from_root(&self.document.items)?, keys)
    }

    pub fn mission_patrol_inputs(
        &self,
        keys: [&[u8]; 3],
    ) -> io::Result<super::naval_missions::PatrolMissionSave> {
        if self.has_repeated_country_tags()? {
            return Err(invalid(
                "repeated country tags require authenticated physical projection for patrol missions",
            ));
        }
        super::naval_missions::patrol_inputs(CountriesView::from_root(&self.document.items)?, keys)
    }

    pub fn mission_spotting_caches(
        &self,
        keys: [&[u8]; 2],
    ) -> io::Result<super::naval_missions::MissionSpottingCaches> {
        if self.has_repeated_country_tags()? {
            return Err(invalid(
                "repeated country tags require authenticated physical projection for mission spotting",
            ));
        }
        super::naval_missions::spotting_caches(
            CountriesView::from_root(&self.document.items)?,
            keys,
        )
    }

    pub fn has_repeated_country_tags(&self) -> io::Result<bool> {
        let mut seen = std::collections::BTreeSet::new();
        for item in self.original_country_items()? {
            let tag = country_tag(item)?;
            block(&item.value)?;
            if !seen.insert(tag) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn project<'a>(
        &'a self,
        layout: &'a [CountryEmissionIdentity],
    ) -> io::Result<CountryProjection<'a>> {
        let items = self.original_country_items()?;
        if items.len() != layout.len() {
            return Err(invalid(format!(
                "saved country emission count {} differs from physical census {}",
                items.len(),
                layout.len()
            )));
        }
        let mut by_actual: BTreeMap<ActualCountry, usize> = BTreeMap::new();
        let mut tag_owners = BTreeMap::new();
        let mut representatives = Vec::new();
        for (ordinal, (item, identity)) in items.iter().zip(layout).enumerate() {
            let physical_slot = u32::try_from(ordinal + 1)
                .map_err(|_| invalid("saved country emission count exceeds u32"))?;
            if identity.physical_slot != physical_slot {
                return Err(invalid(
                    "country emission layout is not full physical-slot order",
                ));
            }
            match identity.actual {
                ActualCountry::Reserved if identity.own_id == 0 => {}
                ActualCountry::Canonical(slot) if slot != 0 && identity.own_id != 0 => {}
                _ => {
                    return Err(invalid(
                        "country emission layout has inconsistent actual identity",
                    ));
                }
            }
            let tag = country_tag(item)?;
            block(&item.value)?;
            if tag != identity.tag.as_slice() {
                return Err(invalid(format!(
                    "country emission {ordinal} at physical slot {physical_slot} has a different tag"
                )));
            }
            if let Some(previous) = tag_owners.insert(tag, identity.actual)
                && previous != identity.actual
            {
                return Err(invalid("distinct actual countries have the same saved tag"));
            }
            if let Some(&first) = by_actual.get(&identity.actual) {
                if items[first] != *item {
                    return Err(invalid(format!(
                        "country emissions {first} and {ordinal} at physical slots {} and {physical_slot} differ at {}",
                        layout[first].physical_slot,
                        first_difference(&items[first], item),
                    )));
                }
            } else {
                by_actual.insert(identity.actual, ordinal);
                representatives.push(ordinal);
            }
        }
        Ok(CountryProjection {
            document: self,
            layout,
            representatives,
        })
    }
}

#[derive(Debug)]
pub struct CountryProjection<'a> {
    document: &'a SaveDocument,
    layout: &'a [CountryEmissionIdentity],
    representatives: Vec<usize>,
}

impl CountryProjection<'_> {
    pub fn mission_patrol_inputs(
        &self,
        keys: [&[u8]; 3],
    ) -> io::Result<super::naval_missions::PatrolMissionSave> {
        let mut countries = CountriesView::from_root(&self.document.document.items)?;
        countries.selected = Some(&self.representatives);
        super::naval_missions::patrol_inputs(countries, keys)
    }

    pub fn mission_raiding_grid(
        &self,
        keys: [&[u8]; 2],
    ) -> io::Result<super::naval_missions::RaidingGridSave> {
        let mut countries = CountriesView::from_root(&self.document.document.items)?;
        countries.selected = Some(&self.representatives);
        super::naval_missions::raiding_grid(countries, keys)
    }

    pub fn mission_spotting_caches(
        &self,
        keys: [&[u8]; 2],
    ) -> io::Result<super::naval_missions::MissionSpottingCaches> {
        let mut countries = CountriesView::from_root(&self.document.document.items)?;
        countries.selected = Some(&self.representatives);
        super::naval_missions::spotting_caches(countries, keys)
    }

    pub fn parse(&self) -> io::Result<Save> {
        let mut countries = CountriesView::from_root(&self.document.document.items)?;
        countries.selected = Some(&self.representatives);
        let mut save = super::parse_items(&self.document.document.items, countries)?;
        save.country_emissions = Some(
            self.document
                .original_country_items()?
                .iter()
                .zip(self.layout)
                .map(|(item, identity)| {
                    Ok(SavedCountryEmission {
                        identity: identity.clone(),
                        intel_present: field(block(&item.value)?, b"intel")?.is_some(),
                    })
                })
                .collect::<io::Result<_>>()?,
        );
        Ok(save)
    }
}

fn country_tag(item: &Item) -> io::Result<&[u8]> {
    item.key
        .as_deref()
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| invalid("country has no tag"))
}

fn first_difference(first: &Item, next: &Item) -> String {
    if first.key != next.key {
        return "country.key".into();
    }
    if first.key_quoted != next.key_quoted {
        return "country.key_quoted".into();
    }
    if first.operator != next.operator {
        return "country.operator".into();
    }
    if first.kind != next.kind {
        return "country.kind".into();
    }
    match (&first.value, &next.value) {
        (Value::Block(a), Value::Block(b)) => {
            for (ordinal, (a, b)) in a.iter().zip(b).enumerate() {
                if a != b {
                    return format!("country[{ordinal}].{}", first_difference(a, b));
                }
            }
            "country.value.length".into()
        }
        _ => "country.value".into(),
    }
}

#[derive(Clone, Copy)]
pub(super) struct CountriesView<'a> {
    root_item: Option<&'a Item>,
    original: Option<&'a [Item]>,
    selected: Option<&'a [usize]>,
}

impl<'a> CountriesView<'a> {
    pub(super) fn from_root(items: &'a [Item]) -> io::Result<Self> {
        let value = field(items, b"countries")?;
        let original = value.map(block).transpose()?;
        let root_item = items
            .iter()
            .find(|item| item.key.as_deref() == Some(b"countries"));
        Ok(Self {
            root_item,
            original,
            selected: None,
        })
    }

    pub(super) fn is_present(self) -> bool {
        self.original.is_some()
    }

    pub(super) fn is_root_item(self, item: &Item) -> bool {
        self.root_item.is_some_and(|root| std::ptr::eq(root, item))
    }

    pub(super) fn required(self) -> io::Result<CountryItems<'a>> {
        if !self.is_present() {
            return Err(invalid("save lacks countries"));
        }
        Ok(self.iter())
    }

    pub(super) fn iter(self) -> CountryItems<'a> {
        CountryItems {
            items: self.original.unwrap_or(&[]),
            selected: self.selected.map(<[usize]>::iter),
            next: 0,
        }
    }
}

pub(super) struct CountryItems<'a> {
    items: &'a [Item],
    selected: Option<slice::Iter<'a, usize>>,
    next: usize,
}

impl<'a> Iterator for CountryItems<'a> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        let index = match &mut self.selected {
            Some(selected) => *selected.next()?,
            None => {
                let index = self.next;
                self.items.get(index)?;
                self.next += 1;
                index
            }
        };
        self.items.get(index)
    }
}

#[cfg(test)]
#[path = "countries/tests.rs"]
mod tests;
