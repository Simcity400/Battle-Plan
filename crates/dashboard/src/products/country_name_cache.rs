//! Cache selected country names through the current shared native resolver.
//! Keep the supplying frame intact, including reserved names and raw tag caches.
use super::operative_rows::{CountryIdentity, country_at};
use installed_content::{
    country_names::{CountryNames, Operands},
    localisation::Localisation,
};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{TableSchema, tables},
};
use std::collections::{BTreeMap, BTreeSet};

fn rows<T: TableSchema>(frame: &DecodedFrame) -> &[T::Row] {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<T>().unwrap_or(&[])
        }
        TableView::Unavailable { .. } => &[],
    }
}

#[derive(Default)]
pub(super) struct CountryNameCache {
    input: BTreeMap<u32, Option<Result<Operands, String>>>,
    slots: BTreeMap<CountryIdentity, bool>,
    reserved_built: Option<u8>,
    names: BTreeMap<CountryIdentity, Result<String, String>>,
}

impl CountryNameCache {
    pub(super) fn update(
        &mut self,
        frame: &DecodedFrame,
        slots: BTreeSet<CountryIdentity>,
        localisation: &Localisation,
    ) -> bool {
        if slots.is_empty() {
            let changed = !self.slots.is_empty();
            self.input.clear();
            self.slots.clear();
            self.names.clear();
            self.reserved_built = None;
            return changed;
        }
        let countries = rows::<tables::Countries>(frame);
        let links = rows::<tables::CountryLinks>(frame);
        let requested: BTreeMap<_, _> = slots
            .iter()
            .map(|&identity| {
                (
                    identity,
                    identity == CountryIdentity { slot: 0, id: 0 }
                        || country_at(identity, countries, links).is_some(),
                )
            })
            .collect();
        // This shared resolver indexes canonical country slots once. Repeated
        // selected-slot reverse searches become quadratic for a large captor census.
        let names = CountryNames::read(frame, localisation);
        let mut pending: Vec<_> = slots.iter().map(|identity| identity.slot).collect();
        let mut input = BTreeMap::new();
        while let Some(slot) = pending.pop() {
            if input.contains_key(&slot) {
                continue;
            }
            let operand = names.operands(slot).cloned();
            if let Some(overlord) = operand
                .as_ref()
                .and_then(|value| value.as_ref().ok())
                .and_then(|value| value.overlord)
            {
                pending.push(overlord);
            }
            input.insert(slot, operand);
        }
        // The zero country's three caches have independent built bits,
        // separate from its generator operands.
        let reserved_built = input
            .contains_key(&0)
            .then(|| {
                frame
                    .complete::<tables::ReservedCountryNaming>()
                    .ok()
                    .and_then(|rows| match rows {
                        [row] => Some(row.built),
                        _ => None,
                    })
            })
            .flatten();
        if self.input == input && self.slots == requested && self.reserved_built == reserved_built {
            return false;
        }
        self.names = requested
            .iter()
            .map(|(&identity, &present)| {
                let name = if present {
                    names
                        .name_with(identity.slot, "_DEF")
                        .map(|name| installed_content::localisation::crt_case_text(&name, true, true))
                        .and_then(|name| crate::catalogue::plain_text(&name))
                } else {
                    Err("Country naming identity is unavailable at this hour".into())
                };
                (identity, name)
            })
            .collect();
        self.input = input;
        self.slots = requested;
        self.reserved_built = reserved_built;
        true
    }

    pub(super) fn name(&self, identity: CountryIdentity) -> Result<&str, &str> {
        match self.names.get(&identity) {
            Some(Ok(name)) => Ok(name),
            Some(Err(reason)) => Err(reason),
            None => Err("Country naming inputs are unavailable at this hour"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::source;
    use replay::rows::{
        AutonomyRow, CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow,
        PoliticsRow, TableId, TableRows,
    };
    #[test]
    fn country_name_cache_ignores_unrelated_values_and_refreshes_naming_operands() {
        let mut frame = source(1, 24000, Some(1));
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 1,
                tag: b"synthetic".to_vec(),
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 7,
                canonical_slot: 1,
            }]))
            .into();
        frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 7,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 7,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![])).into();
        frame.tables[TableId::Autonomy.index()] =
            TableView::Complete(TableRows::Autonomy(vec![])).into();
        let identity = CountryIdentity { id: 7, slot: 1 };
        let loc = [
            ("synthetic_DEF".to_owned(), "old name".to_owned()),
            ("changed_DEF".to_owned(), "new name".to_owned()),
        ]
        .into_iter()
        .collect();
        let mut cache = CountryNameCache::default();
        assert!(cache.update(&frame, BTreeSet::from([identity]), &loc));
        assert_eq!(cache.name(identity), Ok("Old name"));
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].command_power_raw += 1;
        }
        assert!(!cache.update(&frame, BTreeSet::from([identity]), &loc));
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].red += 1;
        }
        assert!(!cache.update(&frame, BTreeSet::from([identity]), &loc));
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].cosmetic_tag = b"changed".to_vec();
        }
        assert!(cache.update(&frame, BTreeSet::from([identity]), &loc));
        assert_eq!(cache.name(identity), Ok("New name"));
    }

    #[test]
    fn country_name_cache_matches_full_names_across_subject_chains_and_missing_inputs() {
        let identity = CountryIdentity { id: 7, slot: 1 };
        let mut frame = source(1, 24000, None);
        frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [(7, 1, "inner"), (17, 2, "middle"), (27, 3, "outer")]
                .into_iter()
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            [(7, 1), (17, 2), (27, 3), (1017, 2), (2027, 3)]
                .into_iter()
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect(),
        ))
        .into();
        frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(
                [(7, 1017), (17, 2027), (27, 0)]
                    .into_iter()
                    .map(|(country, overlord)| CountryAppearanceRow {
                        country,
                        overlord,
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
            [7, 17, 27]
                .into_iter()
                .map(|country| PoliticsRow {
                    country,
                    ruling_party: Some(41),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(
                [7, 17, 27]
                    .into_iter()
                    .map(|country| PoliticalPartyRow {
                        country,
                        id: 41,
                        token: b"mod_group".to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            [7, 17]
                .into_iter()
                .map(|subject| AutonomyRow {
                    subject,
                    state: b"mod_state".to_vec(),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        let localisation: Localisation = [
            ("inner_DEF", "inner"),
            ("middle_DEF", "middle"),
            (
                "inner_middle_subject_DEF",
                "$NONIDEOLOGYDEF$ of $OVERLORDNAMEDEF$",
            ),
            (
                "middle_outer_subject_DEF",
                "$NONIDEOLOGYDEF$ of $OVERLORDNAMEDEF$",
            ),
            ("outer_mod_group_DEF", "outer old"),
            ("outer_alt_group_DEF", "outer changed"),
            ("renamed_outer_alt_group_DEF", "renamed outer"),
            ("inner_middle_special_DEF", "special $OVERLORDNAMEDEF$"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let mut cache = CountryNameCache::default();
        let compare = |cache: &mut CountryNameCache, frame: &DecodedFrame| {
            assert!(cache.update(frame, BTreeSet::from([identity]), &localisation));
            let expected = CountryNames::read(frame, &localisation)
                .name_with(identity.slot, "_DEF")
                .map(|name| installed_content::localisation::crt_case_text(&name, true, true))
                .and_then(|name| crate::catalogue::plain_text(&name));
            assert_eq!(
                cache
                    .name(identity)
                    .map(str::to_owned)
                    .map_err(str::to_owned),
                expected
            );
        };
        compare(&mut cache, &frame);
        let original = cache.name(identity).unwrap().to_owned();
        if let TableView::Complete(TableRows::PoliticalParties(rows)) =
            &mut *frame.tables[TableId::PoliticalParties.index()]
        {
            rows[2].token = b"alt_group".to_vec();
        }
        compare(&mut cache, &frame);
        assert_ne!(cache.name(identity).unwrap(), original);
        let group_name = cache.name(identity).unwrap().to_owned();
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[2].name_tag = b"renamed_outer".to_vec();
        }
        compare(&mut cache, &frame);
        assert_ne!(cache.name(identity).unwrap(), group_name);
        if let TableView::Complete(TableRows::Autonomy(rows)) =
            &mut *frame.tables[TableId::Autonomy.index()]
        {
            rows[0].state = b"special".to_vec();
        }
        compare(&mut cache, &frame);
        assert!(cache.name(identity).unwrap().starts_with("Special "));
        let parties = rows::<tables::PoliticalParties>(&frame)[..2].to_vec();
        frame.tables[TableId::PoliticalParties.index()] = TableView::Truncated {
            rows: TableRows::PoliticalParties(parties),
            missing: 1,
        }
        .into();
        compare(&mut cache, &frame);
        assert_eq!(
            cache.name(identity),
            Err("PoliticalParties is incomplete this hour")
        );
    }

    #[test]
    fn reserved_zero_built_changes_do_not_rebuild_positive_original_empty_names() {
        use replay::rows::{Recorded, ReservedCountryNamingRow};
        let mut frame = source(1, 24000, None);
        frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"ZERO".to_vec(),
                    original_tag: Recorded::Value(Some(Vec::new())),
                    original_tag_reference: Recorded::Value(Some(7)),
                    original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![7, 7]])),
                    country_array: Recorded::Value(Some(vec![0, 7, 7])),
                    ..Default::default()
                },
            ]))
            .into();
        let identity = CountryIdentity { slot: 0, id: 0 };
        let localisation = [("ZERO_DEF".into(), "the reserved name".into())]
            .into_iter()
            .collect();
        let mut cache = CountryNameCache::default();
        assert!(cache.update(&frame, BTreeSet::from([identity]), &localisation));
        assert_eq!(cache.name(identity), Ok(""));
        assert_eq!(
            installed_content::country_names::original_or_own_tag(&frame, 0),
            Ok("")
        );
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].built = 2;
        }
        assert!(cache.update(&frame, BTreeSet::from([identity]), &localisation));
        assert_eq!(cache.name(identity), Ok("The reserved name"));
        assert_eq!(
            installed_content::country_names::original_or_own_tag(&frame, 0),
            Ok("")
        );
        let reserved = frame.complete::<tables::ReservedCountryNaming>().unwrap();
        assert_eq!(
            reserved[0].original_tag_buckets,
            Recorded::Value(Some(vec![vec![], vec![7, 7]]))
        );
        assert_eq!(
            reserved[0].country_array,
            Recorded::Value(Some(vec![0, 7, 7]))
        );
    }

    #[test]
    fn name_cache_refuses_replaced_holder_and_incomplete_country_catalogue() {
        let mut frame = source(1, 24000, None);
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 1,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 7,
                canonical_slot: 1,
            }]))
            .into();
        frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 7,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 7,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::Autonomy.index()] =
            TableView::Complete(TableRows::Autonomy(vec![])).into();
        let localisation = [("SYN_DEF".into(), "synthetic".into())]
            .into_iter()
            .collect();
        let identity = CountryIdentity { slot: 1, id: 7 };
        let mut cache = CountryNameCache::default();
        assert!(cache.update(&frame, BTreeSet::from([identity]), &localisation));
        assert_eq!(cache.name(identity), Ok("Synthetic"));
        let countries = rows::<tables::Countries>(&frame).to_vec();
        frame.tables[TableId::Countries.index()] = TableView::Truncated {
            rows: TableRows::Countries(countries),
            missing: 1,
        }
        .into();
        assert!(cache.update(&frame, BTreeSet::from([identity]), &localisation));
        assert!(cache.name(identity).is_err());
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 17,
                canonical_slot: 1,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 17,
                canonical_slot: 1,
            }]))
            .into();
        assert!(cache.update(&frame, BTreeSet::from([identity]), &localisation));
        assert_eq!(
            cache.name(identity),
            Err("Country naming identity is unavailable at this hour")
        );
    }
}
