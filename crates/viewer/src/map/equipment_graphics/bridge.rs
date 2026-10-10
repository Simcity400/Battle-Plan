//! Replay and matching-content operands for conventional land equipment models.
//!
//! `legacy` implements the native candidate law. This bridge joins the complete
//! saved variant census, each country's three recorded rosters (primary,
//! foreign and stored best-variant candidates), upgrade bytes and
//! runtime country references to authored equipment definitions.

use std::collections::BTreeMap;

use replay::{
    frame::DecodedFrame,
    rows::{
        AvailableEquipmentVariantRow, CountryLinkRow, TableSchema, VariantDefinitionRow,
        VariantUpgradeRow, tables,
    },
};

use super::{Candidate, legacy};
use crate::map::{
    model_countries::ModelCountry,
    model_selection::{Content, SubUnit},
    tables::rows,
};

pub struct Prepared<'a> {
    variants: Vec<legacy::Variant<'a>>,
    /// Indexed by the complete raw VariantDefinitions census. A malformed
    /// variant with a known, unrelated archetype cannot suppress land models.
    states: Vec<VariantState>,
    invalid_by_archetype: BTreeMap<&'a str, usize>,
    invalid_unknown: Option<usize>,
    by_handle: BTreeMap<u64, usize>,
    /// (canonical country slot, roster kind) -> native-order raw variant
    /// indices, repeats included. Kind 0 is primary, 1 foreign, 2 the stored
    /// best-variant candidates.
    available: BTreeMap<(u32, u8), Vec<usize>>,
    links: BTreeMap<u32, u32>,
}

pub struct Selection<'a> {
    pub subunit: &'a SubUnit,
    pub first_archetype: Option<&'a str>,
    pub variant: Option<u64>,
}

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

/// The stored best-variant candidate roster.
const BEST_CANDIDATES: u8 = 2;

impl<'a> Prepared<'a> {
    /// The native A/B roster for the template fallback. Invalid definitions for
    /// any required archetype make the figure unknown.
    pub(crate) fn no_pair_variants(
        &self,
        country_ref: u32,
        required_archetypes: &std::collections::BTreeSet<&str>,
    ) -> Result<Vec<legacy::Variant<'a>>, String> {
        if let Some(index) = self.invalid_unknown {
            return Err(self.states[index]
                .error()
                .expect("indexed invalid variant")
                .to_owned());
        }
        for archetype in required_archetypes {
            if let Some(&index) = self.invalid_by_archetype.get(archetype) {
                return Err(self.states[index]
                    .error()
                    .expect("indexed invalid variant")
                    .to_owned());
            }
        }
        let slot = self.canonical_slot(country_ref)?;
        let mut roster = Vec::new();
        for kind in [0, 1] {
            for &index in self.available.get(&(slot, kind)).into_iter().flatten() {
                match &self.states[index] {
                    VariantState::Ready(ready) => roster.push(self.variants[*ready]),
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
    ) -> Result<Vec<legacy::Variant<'a>>, String> {
        self.listed(country_ref, 0)
    }

    /// The country's recorded best-variant candidates.
    pub(crate) fn best_candidates(
        &self,
        country_ref: u32,
    ) -> Result<Vec<legacy::Variant<'a>>, String> {
        self.listed(country_ref, BEST_CANDIDATES)
    }

    fn listed(&self, country_ref: u32, kind: u8) -> Result<Vec<legacy::Variant<'a>>, String> {
        let slot = self.canonical_slot(country_ref)?;
        self.available
            .get(&(slot, kind))
            .into_iter()
            .flatten()
            .map(|&index| match &self.states[index] {
                VariantState::Ready(ready) => Ok(self.variants[*ready]),
                VariantState::InvalidKnown { error } | VariantState::InvalidUnknown { error } => {
                    Err(error.clone())
                }
            })
            .collect()
    }

    pub(crate) fn canonical_slot_for(&self, reference: u32) -> Result<u32, String> {
        self.canonical_slot(reference)
    }

    /// Only complete tables establish a complete model census.
    pub fn read(frame: &'a DecodedFrame, content: &'a Content) -> Result<Self, String> {
        let variants = complete::<tables::VariantDefinitions>(frame)?;
        let upgrades = complete::<tables::VariantUpgrades>(frame)?;
        let available = complete::<tables::AvailableEquipmentVariants>(frame)?;
        let links = complete::<tables::CountryLinks>(frame)?;
        let features = complete::<tables::SimulationFeatures>(frame)?;
        let [features] = features else {
            return Err("SimulationFeatures must have one complete row".into());
        };
        let module_categories = enabled_module_categories(features.dlc_mask);
        let tokens =
            crate::table_access::TokenNames::new(frame).map_err(|error| error.to_string())?;
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
        tokens: &crate::table_access::TokenNames<'_>,
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
        for row in definitions {
            match build_variant(
                content,
                tokens,
                row,
                &links,
                &upgrade_sums,
                module_categories,
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
                            invalid_by_archetype.entry(archetype).or_insert(index);
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
        let available = join_available(available, &by_handle, &links)?;
        Ok(Self {
            variants,
            states,
            invalid_by_archetype,
            invalid_unknown,
            by_handle,
            available,
            links,
        })
    }

    /// Return the displayed entity only when the recorded and authored inputs
    /// determine one. The current `country_ref` can differ from the variant's
    /// creator and from the roster owner used earlier to choose this model.
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
            if let Some(&index) = self.invalid_by_archetype.get(archetype) {
                return Err(self.states[index]
                    .error()
                    .expect("indexed invalid variant")
                    .to_owned());
            }
            if let Some(index) = self.invalid_unknown {
                return Err(self.states[index]
                    .error()
                    .expect("indexed invalid variant")
                    .to_owned());
            }
        }
        // An invalid definition of another archetype cannot reach this
        // subunit's candidates; one of its own archetype refused above.
        let ready = |kind: u8| -> Vec<_> {
            self.available
                .get(&(slot, kind))
                .into_iter()
                .flatten()
                .filter_map(|&index| match &self.states[index] {
                    VariantState::Ready(index) => Some(self.variants[*index]),
                    VariantState::InvalidKnown { .. } | VariantState::InvalidUnknown { .. } => None,
                })
                .collect()
        };
        let available = ready(0);
        let best = ready(BEST_CANDIDATES);
        let selected = selected_variant
            .map(|handle| {
                let index = *self
                    .by_handle
                    .get(&handle)
                    .ok_or_else(|| format!("selected equipment variant {handle} is unavailable"))?;
                match &self.states[index] {
                    VariantState::Ready(index) => Ok(&self.variants[*index]),
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
                selected,
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
        self.links
            .get(&reference)
            .copied()
            .ok_or_else(|| format!("model country {reference} has no CountryLinks row"))
    }
}

fn build_variant<'a>(
    content: &'a Content,
    tokens: &crate::table_access::TokenNames<'_>,
    row: &'a VariantDefinitionRow,
    links: &BTreeMap<u32, u32>,
    upgrade_sums: &BTreeMap<u64, i32>,
    module_categories: u64,
) -> Result<legacy::Variant<'a>, String> {
    let key = content.equipment_key(tokens, row)?;
    let model = content
        .equipment_model(key)
        .map_err(|error| format!("variant {}: {error}", row.id))?;
    let creator_reference = row.creator_link as i32;
    // A nonpositive reference has no canonical country object. Give a
    // negative sentinel its own slot to avoid falsely aliasing another
    // nonpositive reference through canonical slot zero.
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
    let has_enabled_module_slots = content
        .has_enabled_module_slots(key, module_categories)
        .map_err(|error| format!("variant {}: {error}", row.id))?;
    Ok(legacy::Variant {
        handle: row.id,
        // A variant handle's low word is the reference type;
        // the persistent object ID lives in its upper 32 bits.
        id: (row.id >> 32) as u32,
        creator_reference,
        creator_slot,
        upgrade_sum: upgrade_sums.get(&row.id).copied().unwrap_or_default(),
        equipment: legacy::Equipment {
            archetype: model.archetype,
            archetype_ordinal: model.archetype_ordinal,
            sprite: model.model,
            visual_level: model.visual_level,
            model_priority: model.model_priority,
            parent_depth: model.depth,
            has_enabled_module_slots,
        },
    })
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
        // The game sums unsigned level bytes with wrapping i32 arithmetic.
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

/// Equipment categories check three command feature bits in the saved
/// simulation DLC mask
/// rather than intersecting raw DLC bits with equipment types.
fn enabled_module_categories(dlc_mask: u32) -> u64 {
    let mut categories = 0;
    if dlc_mask & 0x10 != 0 {
        categories |= 0x80_0040_01c0;
    }
    if dlc_mask & 0x80 != 0 {
        categories |= 0x4;
    }
    if dlc_mask & 0x100 != 0 {
        categories |= 0x1d_0026_fc00;
    }
    categories
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
            variants,
            states,
            invalid_by_archetype: BTreeMap::new(),
            invalid_unknown: None,
            by_handle,
            available: BTreeMap::from([((1, 0), vec![0, 1])]),
            links: BTreeMap::from([(1, 1), (10, 1)]),
        }
    }

    #[test]
    fn saved_feature_bits_map_to_equipment_categories_not_identical_bits() {
        assert_eq!(enabled_module_categories(0), 0);
        assert_eq!(enabled_module_categories(0x10), 0x80_0040_01c0);
        assert_eq!(enabled_module_categories(0x80), 0x4);
        assert_eq!(enabled_module_categories(0x100), 0x1d_0026_fc00);
        assert_eq!(
            enabled_module_categories(0x190),
            0x80_0040_01c0 | 0x4 | 0x1d_0026_fc00
        );
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        bridge.available.insert((1, 1), vec![1, 0, 1]);
        bridge.available.insert((1, BEST_CANDIDATES), vec![1]);
        let roster = bridge
            .no_pair_variants(10, &std::collections::BTreeSet::from(["rifles"]))
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // Without a selected variant, the recorded best entry decides.
        for (best, winner) in [(0, "GER_infantry_1_entity"), (1, "GER_infantry_3_entity")] {
            bridge.available.insert((1, BEST_CANDIDATES), vec![best]);
            assert_eq!(
                bridge
                    .resolve_operands(10, country, subunit, None, &[], exists)
                    .unwrap()
                    .as_deref(),
                Some(winner)
            );
        }
        // The division path's selected variant carries its own bonus.
        assert_eq!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap()
                .as_deref(),
            Some("GER_infantry_1_entity")
        );
        // A roster kind past the best candidates is refused.
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
        bridge.states.push(VariantState::InvalidKnown {
            error: "variant 3: invalid naval module".into(),
        });
        bridge.invalid_by_archetype.insert("ship_hull_submarine", 2);
        bridge.by_handle.insert(handle(3), 2);
        bridge.available.get_mut(&(1, 0)).unwrap().push(2);
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
        bridge.states[2] = VariantState::InvalidKnown {
            error: "variant 3: invalid rifle module".into(),
        };
        bridge.invalid_by_archetype.clear();
        bridge.invalid_by_archetype.insert("rifles", 2);
        assert!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap_err()
                .contains("invalid rifle module")
        );
        bridge.states[2] = VariantState::InvalidUnknown {
            error: "variant 3: unknown equipment archetype".into(),
        };
        bridge.invalid_by_archetype.clear();
        bridge.invalid_unknown = Some(2);
        assert!(
            bridge
                .resolve_operands(10, country, subunit, Some(handle(1)), &[], exists)
                .unwrap_err()
                .contains("unknown equipment archetype")
        );
    }
}
