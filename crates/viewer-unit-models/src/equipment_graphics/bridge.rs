
use replay_frame::frame::kept;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use dashboard_text::table_access::TokenNames;

use replay_data::{
    rows::AvailableEquipmentVariantRow, rows::CountryLinkRow, rows::TableSchema,
    rows::VariantDefinitionRow, rows::VariantUpgradeRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use content_reader_equipment_graphics::equipment_graphics::Candidate;
use super::legacy;
use crate::model_selection::{Content, EquipmentModel, SubUnit, model_facts};
use content_reader_equipment_graphics::equipment_graphics::countries::ModelCountry;
use viewer_foundation::tables::rows;

pub struct Prepared<'a> {
    variants: Variants<'a>,
    data: Arc<Census>,
}

#[derive(Clone, Debug, PartialEq)]
struct Census {
    states: Vec<VariantState>,
    invalid_by_archetype: BTreeMap<String, usize>,
    invalid_unknown: Option<usize>,
    by_handle: BTreeMap<u64, usize>,
    available: BTreeMap<(u32, u8), Vec<usize>>,
    links: BTreeMap<u32, u32>,
}

enum Variants<'a> {
    Borrowed(Vec<legacy::Variant<'a>>),
    Retained(Arc<[OwnedVariant]>),
}

impl Variants<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Borrowed(rows) => rows.len(),
            Self::Retained(rows) => rows.len(),
        }
    }

    fn get(&self, index: usize) -> legacy::Variant<'_> {
        match self {
            Self::Borrowed(rows) => rows[index],
            Self::Retained(rows) => rows[index].borrowed(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CensusRead {
    Census,
    Unknown,
    Archetype(String),
    Link(u32),
    List(u32, u8),
    Handle(u64),
}

impl kept::Grouped for CensusRead {
    fn table(&self) -> Option<replay_data::rows::TableId> {
        None
    }
}

#[derive(PartialEq)]
pub struct Retained {
    variants: Arc<[OwnedVariant]>,
    data: Arc<Census>,
}

#[derive(Debug, PartialEq)]
struct OwnedVariant {
    handle: u64,
    id: u32,
    creator_reference: i32,
    creator_slot: i32,
    upgrade_sum: i32,
    archetype: String,
    archetype_ordinal: usize,
    sprite: String,
    visual_level: i32,
    model_priority: i32,
    parent_depth: i32,
    has_enabled_module_slots: bool,
}

impl OwnedVariant {
    fn new(row: legacy::Variant<'_>) -> Self {
        let equipment = row.equipment;
        Self {
            handle: row.handle,
            id: row.id,
            creator_reference: row.creator_reference,
            creator_slot: row.creator_slot,
            upgrade_sum: row.upgrade_sum,
            archetype: equipment.archetype.to_owned(),
            archetype_ordinal: equipment.archetype_ordinal,
            sprite: equipment.sprite.to_owned(),
            visual_level: equipment.visual_level,
            model_priority: equipment.model_priority,
            parent_depth: equipment.parent_depth,
            has_enabled_module_slots: equipment.has_enabled_module_slots,
        }
    }

    fn borrowed(&self) -> legacy::Variant<'_> {
        legacy::Variant {
            handle: self.handle,
            id: self.id,
            creator_reference: self.creator_reference,
            creator_slot: self.creator_slot,
            upgrade_sum: self.upgrade_sum,
            equipment: legacy::Equipment {
                archetype: &self.archetype,
                archetype_ordinal: self.archetype_ordinal,
                sprite: &self.sprite,
                visual_level: self.visual_level,
                model_priority: self.model_priority,
                parent_depth: self.parent_depth,
                has_enabled_module_slots: self.has_enabled_module_slots,
            },
        }
    }
}

impl Retained {
    pub fn read(frame: &DecodedFrame, content: &Content) -> Result<Self, String> {
        let prepared = Prepared::read(frame, content)?;
        Ok(Self {
            variants: (0..prepared.variants.len())
                .map(|index| OwnedVariant::new(prepared.variants.get(index)))
                .collect(),
            data: prepared.data,
        })
    }

    pub fn same_read(&self, other: &Self, read: &CensusRead) -> bool {
        fn state(census: &Retained, index: usize) -> Result<&OwnedVariant, &str> {
            match &census.data.states[index] {
                VariantState::Ready(index) => Ok(&census.variants[*index]),
                VariantState::InvalidKnown { error } | VariantState::InvalidUnknown { error } => {
                    Err(error.as_str())
                }
            }
        }
        match read {
            CensusRead::Census => true,
            CensusRead::Unknown => {
                self.data.invalid_unknown.map(|index| state(self, index))
                    == other.data.invalid_unknown.map(|index| state(other, index))
            }
            CensusRead::Archetype(archetype) => {
                fn entry<'c>(
                    census: &'c Retained,
                    archetype: &str,
                ) -> Option<Result<&'c OwnedVariant, &'c str>> {
                    let index = *census.data.invalid_by_archetype.get(archetype)?;
                    Some(state(census, index))
                }
                entry(self, archetype) == entry(other, archetype)
            }
            CensusRead::Link(reference) => {
                self.data.links.get(reference) == other.data.links.get(reference)
            }
            CensusRead::List(slot, kind) => {
                match (
                    self.data.available.get(&(*slot, *kind)),
                    other.data.available.get(&(*slot, *kind)),
                ) {
                    (Some(mine), Some(theirs)) => {
                        mine.len() == theirs.len()
                            && mine
                                .iter()
                                .zip(theirs)
                                .all(|(&a, &b)| state(self, a) == state(other, b))
                    }
                    (None, None) => true,
                    _ => false,
                }
            }
            CensusRead::Handle(handle) => {
                self.data.by_handle.get(handle).map(|&index| state(self, index))
                    == other.data.by_handle.get(handle).map(|&index| state(other, index))
            }
        }
    }

    pub fn prepared(&self) -> Prepared<'static> {
        Prepared {
            variants: Variants::Retained(Arc::clone(&self.variants)),
            data: Arc::clone(&self.data),
        }
    }
}

pub struct Selection<'a> {
    pub subunit: &'a SubUnit,
    pub first_archetype: Option<&'a str>,
    pub variant: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
enum VariantState {
    Ready(usize),
    InvalidKnown { error: String },
    InvalidUnknown { error: String },
}

impl VariantState {
    fn error(&self) -> Option<&str> {
        match self {
            Self::Ready(_) => None,
            Self::InvalidKnown { error, .. } | Self::InvalidUnknown { error } => Some(error),
        }
    }
}

const BEST_CANDIDATES: u8 = 2;

impl<'a> Prepared<'a> {
    #[cfg(test)]
    pub(crate) fn comparison_state(&self) -> String {
        format!("{:?}|{:?}", (0..self.variants.len()).map(|index| self.variants.get(index)).collect::<Vec<_>>(), self.data)
    }

    pub(crate) fn no_pair_variants(
        &self,
        country_ref: u32,
        required_archetypes: &BTreeSet<&str>,
    ) -> Result<Vec<legacy::Variant<'_>>, String> {
        kept::note(|| CensusRead::Unknown);
        for archetype in required_archetypes {
            kept::note(|| CensusRead::Archetype((*archetype).to_owned()));
        }
        if let Some(index) = self.data.invalid_unknown {
            return Err(self.data.states[index]
                .error()
                .expect("indexed invalid variant")
                .to_owned());
        }
        for archetype in required_archetypes {
            if let Some(&index) = self.data.invalid_by_archetype.get(*archetype) {
                return Err(self.data.states[index]
                    .error()
                    .expect("indexed invalid variant")
                    .to_owned());
            }
        }
        let slot = self.canonical_slot(country_ref)?;
        let mut roster = Vec::new();
        for kind in [0, 1] {
            kept::note(|| CensusRead::List(slot, kind));
            for &index in self.data.available.get(&(slot, kind)).into_iter().flatten() {
                match &self.data.states[index] {
                    VariantState::Ready(ready) => roster.push(self.variants.get(*ready)),
                    VariantState::InvalidKnown { error }
                    | VariantState::InvalidUnknown { error } => {
                        return Err(error.clone());
                    }
                }
            }
        }
        Ok(roster)
    }

    pub(crate) fn primary_variants(
        &self,
        country_ref: u32,
    ) -> Result<Vec<legacy::Variant<'_>>, String> {
        self.listed(country_ref, 0)
    }

    pub(crate) fn best_candidates(
        &self,
        country_ref: u32,
    ) -> Result<Vec<legacy::Variant<'_>>, String> {
        self.listed(country_ref, BEST_CANDIDATES)
    }

    fn listed(&self, country_ref: u32, kind: u8) -> Result<Vec<legacy::Variant<'_>>, String> {
        let slot = self.canonical_slot(country_ref)?;
        kept::note(|| CensusRead::List(slot, kind));
        self.data
            .available
            .get(&(slot, kind))
            .into_iter()
            .flatten()
            .map(|&index| match &self.data.states[index] {
                VariantState::Ready(ready) => Ok(self.variants.get(*ready)),
                VariantState::InvalidKnown { error } | VariantState::InvalidUnknown { error } => {
                    Err(error.clone())
                }
            })
            .collect()
    }

    pub(crate) fn canonical_slot_for(&self, reference: u32) -> Result<u32, String> {
        self.canonical_slot(reference)
    }

    pub fn read(frame: &'a DecodedFrame, content: &'a Content) -> Result<Self, String> {
        let variants = complete::<tables::VariantDefinitions>(frame)?;
        let upgrades = complete::<tables::VariantUpgrades>(frame)?;
        let available = complete::<tables::AvailableEquipmentVariants>(frame)?;
        let links = complete::<tables::CountryLinks>(frame)?;
        let features = complete::<tables::SimulationFeatures>(frame)?;
        let [features] = features else {
            return Err("SimulationFeatures must have one complete row".into());
        };
        let module_categories = content_reader_key_names::equipment::category_families()?
            .modular_categories(features.dlc_mask);
        let tokens = TokenNames::new(frame).map_err(|error| error.to_string())?;
        Self::from_rows(
            content,
            &tokens,
            variants,
            upgrades,
            available,
            links,
            module_categories,
        )
    }

    fn from_rows(
        content: &'a Content,
        tokens: &TokenNames<'_>,
        definitions: &'a [VariantDefinitionRow],
        upgrades: &[VariantUpgradeRow],
        available: &[AvailableEquipmentVariantRow],
        links: &[CountryLinkRow],
        module_categories: u64,
    ) -> Result<Self, String> {
        let mut by_handle = BTreeMap::new();
        for (index, row) in definitions.iter().enumerate() {
            if by_handle.insert(row.id, index).is_some() {
                return Err(format!("duplicate equipment variant handle {}", row.id));
            }
        }
        let links: BTreeMap<_, _> = links
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        for row in definitions {
            if (row.creator_link as i32) > 0 && !links.contains_key(&row.creator_link) {
                return Err(format!(
                    "variant {} creator {} has no CountryLinks row",
                    row.id, row.creator_link
                ));
            }
        }
        let upgrade_sums = sum_upgrades(upgrades, &by_handle)?;
        let mut variants = Vec::with_capacity(definitions.len());
        let mut states = Vec::with_capacity(definitions.len());
        let mut invalid_by_archetype = BTreeMap::new();
        let mut invalid_unknown = None;
        let mut facts = content.model_facts().census(module_categories);
        for row in definitions {
            match build_variant(
                content,
                tokens,
                row,
                &links,
                &upgrade_sums,
                &mut facts,
            ) {
                Ok(variant) => {
                    states.push(VariantState::Ready(variants.len()));
                    variants.push(variant);
                }
                Err(error) => {
                    let archetype = content
                        .equipment_key(tokens, row)
                        .ok()
                        .and_then(|key| content.archetype(key).ok());
                    let index = states.len();
                    states.push(match archetype {
                        Some(archetype) => {
                            invalid_by_archetype
                                .entry(archetype.to_owned())
                                .or_insert(index);
                            VariantState::InvalidKnown { error }
                        }
                        None => {
                            invalid_unknown.get_or_insert(index);
                            VariantState::InvalidUnknown { error }
                        }
                    });
                }
            }
        }
        facts.commit();
        let available = join_available(available, &by_handle, &links)?;
        Ok(Self {
            variants: Variants::Borrowed(variants),
            data: Arc::new(Census {
                states,
                invalid_by_archetype,
                invalid_unknown,
                by_handle,
                available,
                links,
            }),
        })
    }

    pub fn resolve(
        &self,
        country_ref: u32,
        country: &ModelCountry,
        selection: Selection<'_>,
        graphical: &[Candidate],
        entity_exists: impl FnMut(&str) -> bool,
    ) -> Result<Option<String>, String> {
        let culture = country.graphical_culture.as_deref().ok_or_else(|| {
            format!("country {country_ref} graphical culture bytes are unavailable")
        })?;
        self.resolve_operands(
            country_ref,
            legacy::Country {
                tag: &country.tag,
                cosmetic: &country.cosmetic,
                graphical_culture: culture,
                reference: country_ref as i32,
                canonical_slot: self.canonical_slot(country_ref)? as i32,
            },
            legacy::Subunit {
                key: &selection.subunit.key,
                sprite: &selection.subunit.sprite,
                categories: selection.subunit.category,
                first_archetype: selection.first_archetype,
            },
            selection.variant,
            graphical,
            entity_exists,
        )
    }

    fn resolve_operands(
        &self,
        country_ref: u32,
        country: legacy::Country<'_>,
        subunit: legacy::Subunit<'_>,
        selected_variant: Option<u64>,
        graphical: &[Candidate],
        entity_exists: impl FnMut(&str) -> bool,
    ) -> Result<Option<String>, String> {
        let slot = self.canonical_slot(country_ref)?;
        if let Some(archetype) = subunit.first_archetype {
            kept::note(|| CensusRead::Archetype(archetype.to_owned()));
            kept::note(|| CensusRead::Unknown);
            if let Some(&index) = self.data.invalid_by_archetype.get(archetype) {
                return Err(self.data.states[index]
                    .error()
                    .expect("indexed invalid variant")
                    .to_owned());
            }
            if let Some(index) = self.data.invalid_unknown {
                return Err(self.data.states[index]
                    .error()
                    .expect("indexed invalid variant")
                    .to_owned());
            }
        }
        let ready = |kind: u8| -> Vec<_> {
            self.data
                .available
                .get(&(slot, kind))
                .into_iter()
                .flatten()
                .filter_map(|&index| match &self.data.states[index] {
                    VariantState::Ready(index) => Some(self.variants.get(*index)),
                    VariantState::InvalidKnown { .. } | VariantState::InvalidUnknown { .. } => None,
                })
                .collect()
        };
        kept::note(|| CensusRead::List(slot, 0));
        kept::note(|| CensusRead::List(slot, BEST_CANDIDATES));
        let available = ready(0);
        let best = ready(BEST_CANDIDATES);
        let selected = selected_variant
            .map(|handle| {
                kept::note(|| CensusRead::Handle(handle));
                let index = *self
                    .data
                    .by_handle
                    .get(&handle)
                    .ok_or_else(|| format!("selected equipment variant {handle} is unavailable"))?;
                match &self.data.states[index] {
                    VariantState::Ready(index) => Ok(self.variants.get(*index)),
                    VariantState::InvalidKnown { error, .. }
                    | VariantState::InvalidUnknown { error } => Err(error.clone()),
                }
            })
            .transpose()?;
        let mut candidates = graphical.to_vec();
        legacy::append_candidates(
            &legacy::Context {
                country,
                subunit,
                selected: selected.as_ref(),
                available: &available,
                best_candidates: &best,
            },
            &mut candidates,
            entity_exists,
        )?;
        Ok(candidates
            .into_iter()
            .next()
            .map(|candidate| candidate.entity))
    }

    fn canonical_slot(&self, reference: u32) -> Result<u32, String> {
        kept::note(|| CensusRead::Link(reference));
        self.data
            .links
            .get(&reference)
            .copied()
            .ok_or_else(|| format!("model country {reference} has no CountryLinks row"))
    }
}

enum Authored<'a> {
    Held(model_facts::Fact),
    Derived(EquipmentModel<'a>),
}

fn build_variant<'a>(
    content: &'a Content,
    tokens: &TokenNames<'_>,
    row: &'a VariantDefinitionRow,
    links: &BTreeMap<u32, u32>,
    upgrade_sums: &BTreeMap<u64, i32>,
    facts: &mut model_facts::View<'a>,
) -> Result<legacy::Variant<'a>, String> {
    let key = content.equipment_key(tokens, row)?;
    let authored = match facts.get(key) {
        Some(fact) => Authored::Held(fact),
        None => Authored::Derived(
            content
                .equipment_model(key)
                .map_err(|error| format!("variant {}: {error}", row.id))?,
        ),
    };
    let creator_reference = row.creator_link as i32;
    let creator_slot = if creator_reference > 0 {
        *links.get(&row.creator_link).ok_or_else(|| {
            format!(
                "variant {} creator {} has no CountryLinks row",
                row.id, row.creator_link
            )
        })? as i32
    } else {
        creator_reference
    };
    let equipment = match authored {
        Authored::Held(fact) => held_equipment(content, fact),
        Authored::Derived(model) => {
            let has_enabled_module_slots = content
                .has_enabled_module_slots(key, facts.module_categories())
                .map_err(|error| format!("variant {}: {error}", row.id))?;
            let at = content.equipment_content().get(key);
            facts.insert(
                key,
                model_facts::Fact {
                    equipment: at.expect("resolved authored equipment"),
                    archetype: model.archetype_ordinal,
                    visual_level: model.visual_level,
                    model_priority: model.model_priority,
                    depth: model.depth,
                    has_enabled_module_slots,
                },
            );
            legacy::Equipment {
                archetype: model.archetype,
                archetype_ordinal: model.archetype_ordinal,
                sprite: model.model,
                visual_level: model.visual_level,
                model_priority: model.model_priority,
                parent_depth: model.depth,
                has_enabled_module_slots,
            }
        }
    };
    Ok(legacy::Variant {
        handle: row.id,
        id: (row.id >> 32) as u32,
        creator_reference,
        creator_slot,
        upgrade_sum: upgrade_sums.get(&row.id).copied().unwrap_or_default(),
        equipment,
    })
}

fn held_equipment(content: &Content, fact: model_facts::Fact) -> legacy::Equipment<'_> {
    let equipment = &content.equipment_content().equipment;
    legacy::Equipment {
        archetype: equipment[fact.archetype]
            .name_str()
            .expect("validated archetype spelling"),
        archetype_ordinal: fact.archetype,
        sprite: content_reader_parser::parser::text(&equipment[fact.equipment].model)
            .expect("validated model spelling"),
        visual_level: fact.visual_level,
        model_priority: fact.model_priority,
        parent_depth: fact.depth,
        has_enabled_module_slots: fact.has_enabled_module_slots,
    }
}

fn complete<T: TableSchema>(frame: &DecodedFrame) -> Result<&[T::Row], String> {
    let view = rows::<T>(frame).map_err(|error| error.to_string())?;
    if view.is_complete() {
        Ok(view.rows())
    } else {
        Err(format!(
            "{:?} is not complete for hour {}",
            T::ID,
            frame.hour
        ))
    }
}

fn sum_upgrades(
    rows: &[VariantUpgradeRow],
    by_handle: &BTreeMap<u64, usize>,
) -> Result<BTreeMap<u64, i32>, String> {
    let mut sums = BTreeMap::<u64, i32>::new();
    for row in rows {
        if !by_handle.contains_key(&row.variant) {
            return Err(format!(
                "upgrade references missing equipment variant {}",
                row.variant
            ));
        }
        let sum = sums.entry(row.variant).or_default();
        *sum = sum.wrapping_add(i32::from(row.level));
    }
    Ok(sums)
}

fn join_available(
    rows: &[AvailableEquipmentVariantRow],
    by_handle: &BTreeMap<u64, usize>,
    links: &BTreeMap<u32, u32>,
) -> Result<BTreeMap<(u32, u8), Vec<usize>>, String> {
    let mut ordered = BTreeMap::<(u32, u8), Vec<(u32, usize)>>::new();
    for row in rows {
        if row.roster_kind > BEST_CANDIDATES {
            return Err(format!(
                "country {} has invalid equipment roster kind {}",
                row.country, row.roster_kind
            ));
        }
        let slot = *links.get(&row.country).ok_or_else(|| {
            format!(
                "available equipment country {} has no CountryLinks row",
                row.country
            )
        })?;
        let index = *by_handle.get(&row.variant).ok_or_else(|| {
            format!(
                "country {} availability references missing variant {}",
                row.country, row.variant
            )
        })?;
        ordered
            .entry((slot, row.roster_kind))
            .or_default()
            .push((row.ordinal, index));
    }
    let mut result = BTreeMap::new();
    for ((slot, kind), mut entries) in ordered {
        entries.sort_by_key(|(ordinal, _)| *ordinal);
        if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(format!(
                "country slot {slot} roster {kind} has repeated availability ordinals"
            ));
        }
        result.insert(
            (slot, kind),
            entries.into_iter().map(|(_, index)| index).collect(),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(id: u32) -> u64 {
        (u64::from(id) << 32) | 70
    }

    fn variant(id: u32, visual_level: i32) -> legacy::Variant<'static> {
        legacy::Variant {
            handle: handle(id),
            id,
            creator_reference: 1,
            creator_slot: 1,
            upgrade_sum: 0,
            equipment: legacy::Equipment {
                archetype: "rifles",
                archetype_ordinal: 0,
                sprite: "",
                visual_level,
                model_priority: visual_level,
                parent_depth: visual_level,
                has_enabled_module_slots: false,
            },
        }
    }

    fn prepared(variants: Vec<legacy::Variant<'static>>) -> Prepared<'static> {
        let by_handle = variants
            .iter()
            .enumerate()
            .map(|(index, variant)| (variant.handle, index))
            .collect();
        let states = (0..variants.len()).map(VariantState::Ready).collect();
        Prepared {
            variants: Variants::Borrowed(variants),
            data: Arc::new(Census {
                states,
                invalid_by_archetype: BTreeMap::new(),
                invalid_unknown: None,
                by_handle,
                available: BTreeMap::from([((1, 0), vec![0, 1])]),
                links: BTreeMap::from([(1, 1), (10, 1)]),
            }),
        }
    }

    #[test]
    fn unsigned_upgrade_levels_sum_and_missing_census_is_an_error() {
        let by_handle = BTreeMap::from([(handle(2), 0)]);
        let rows = vec![
            VariantUpgradeRow {
                variant: handle(2),
                level: 255,
                ..Default::default()
            },
            VariantUpgradeRow {
                variant: handle(2),
                level: 4,
                ..Default::default()
            },
        ];
        assert_eq!(sum_upgrades(&rows, &by_handle).unwrap()[&handle(2)], 259);
        let mut missing = rows;
        missing[1].variant = handle(3);
        assert!(sum_upgrades(&missing, &by_handle).is_err());
    }

    #[test]
    fn availability_preserves_native_order_and_duplicate_variant_membership() {
        content_reader_script::test_game::enter();
        let by_handle = BTreeMap::from([(handle(1), 0), (handle(2), 1)]);
        let links = BTreeMap::from([(10, 1), (11, 1)]);
        let rows = vec![
            AvailableEquipmentVariantRow {
                country: 10,
                roster_kind: 0,
                ordinal: 2,
                variant: handle(1),
            },
            AvailableEquipmentVariantRow {
                country: 10,
                roster_kind: 0,
                ordinal: 0,
                variant: handle(2),
            },
            AvailableEquipmentVariantRow {
                country: 10,
                roster_kind: 0,
                ordinal: 1,
                variant: handle(2),
            },
            AvailableEquipmentVariantRow {
                country: 10,
                roster_kind: 1,
                ordinal: 1,
                variant: handle(1),
            },
            AvailableEquipmentVariantRow {
                country: 10,
                roster_kind: 1,
                ordinal: 0,
                variant: handle(2),
            },
        ];
        assert_eq!(
            join_available(&rows, &by_handle, &links).unwrap()[&(1, 0)],
            vec![1, 1, 0]
        );
        assert_eq!(
            join_available(&rows, &by_handle, &links).unwrap()[&(1, 1)],
            vec![1, 0]
        );
        let mut missing = rows;
        missing[1].variant = handle(3);
        assert!(join_available(&missing, &by_handle, &links).is_err());
    }

    #[test]
    fn no_pair_roster_concatenates_primary_and_foreign_before_native_sort() {
        let mut bridge = prepared(vec![variant(1, 1), variant(2, 2)]);
        Arc::make_mut(&mut bridge.data)
            .available
            .insert((1, 1), vec![1, 0, 1]);
        Arc::make_mut(&mut bridge.data)
            .available
            .insert((1, BEST_CANDIDATES), vec![1]);
        let roster = bridge
            .no_pair_variants(10, &BTreeSet::from(["rifles"]))
            .unwrap();
        assert_eq!(
            roster.iter().map(|variant| variant.id).collect::<Vec<_>>(),
            [1, 2, 2, 1, 2]
        );
        assert_eq!(bridge.primary_variants(10).unwrap().len(), 2);
        assert_eq!(bridge.best_candidates(10).unwrap()[0].id, 2);
    }

    #[test]
    fn the_recorded_best_candidates_decide_the_conventional_winner() {
        content_reader_script::test_game::enter();
        let mut bridge = prepared(vec![variant(1, 1), variant(2, 3)]);
        let country = legacy::Country {
            tag: "GER",
            cosmetic: "",
            graphical_culture: "western",
            reference: 10,
            canonical_slot: 1,
        };
        let subunit = legacy::Subunit {
            key: "infantry",
            sprite: "infantry",
            categories: 0x20,
            first_archetype: Some("rifles"),
        };
        let exists =
            |entity: &str| matches!(entity, "GER_infantry_1_entity" | "GER_infantry_3_entity");
        for (best, winner) in [(0, "GER_infantry_1_entity"), (1, "GER_infantry_3_entity")] {
            Arc::make_mut(&mut bridge.data)
                .available
                .insert((1, BEST_CANDIDATES), vec![best]);
            assert_eq!(
                bridge
                    .resolve_operands(10, country, subunit, None, &[], exists)
                    .unwrap()
                    .as_deref(),
                Some(winner)
            );
        }
        assert_eq!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap()
                .as_deref(),
            Some("GER_infantry_1_entity")
        );
        let links = BTreeMap::from([(10, 1)]);
        let by_handle = BTreeMap::from([(handle(1), 0)]);
        let row = AvailableEquipmentVariantRow {
            country: 10,
            roster_kind: BEST_CANDIDATES + 1,
            ordinal: 0,
            variant: handle(1),
        };
        assert!(join_available(&[row], &by_handle, &links).is_err());
    }

    #[test]
    fn invalid_variant_only_blocks_its_archetype_or_an_explicit_selection() {
        let mut bridge = prepared(vec![variant(1, 1), variant(2, 3)]);
        Arc::make_mut(&mut bridge.data)
            .states
            .push(VariantState::InvalidKnown {
                error: "variant 3: invalid naval module".into(),
            });
        Arc::make_mut(&mut bridge.data)
            .invalid_by_archetype
            .insert("ship_hull_submarine".into(), 2);
        Arc::make_mut(&mut bridge.data)
            .by_handle
            .insert(handle(3), 2);
        Arc::make_mut(&mut bridge.data)
            .available
            .get_mut(&(1, 0))
            .unwrap()
            .push(2);
        let country = legacy::Country {
            tag: "GER",
            cosmetic: "",
            graphical_culture: "western",
            reference: 10,
            canonical_slot: 1,
        };
        let subunit = legacy::Subunit {
            key: "infantry",
            sprite: "infantry",
            categories: 0x20,
            first_archetype: Some("rifles"),
        };
        let exists =
            |entity: &str| matches!(entity, "GER_infantry_1_entity" | "GER_infantry_3_entity");
        assert_eq!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap()
                .as_deref(),
            Some("GER_infantry_1_entity")
        );
        assert!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(3)), &[], exists)
                .unwrap_err()
                .contains("invalid naval module")
        );
        Arc::make_mut(&mut bridge.data).states[2] = VariantState::InvalidKnown {
            error: "variant 3: invalid rifle module".into(),
        };
        Arc::make_mut(&mut bridge.data).invalid_by_archetype.clear();
        Arc::make_mut(&mut bridge.data)
            .invalid_by_archetype
            .insert("rifles".into(), 2);
        assert!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap_err()
                .contains("invalid rifle module")
        );
        Arc::make_mut(&mut bridge.data).states[2] = VariantState::InvalidUnknown {
            error: "variant 3: unknown equipment archetype".into(),
        };
        Arc::make_mut(&mut bridge.data).invalid_by_archetype.clear();
        Arc::make_mut(&mut bridge.data).invalid_unknown = Some(2);
        assert!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap_err()
                .contains("unknown equipment archetype")
        );
    }
}

#[cfg(test)]
#[path = "bridge_reference.rs"]
pub(crate) mod reference;
