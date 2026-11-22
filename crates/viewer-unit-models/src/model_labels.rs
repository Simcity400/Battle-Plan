
use content_reader_localisation::localisation::Localisation;
use replay_data::{country_flags, rows::VariantDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};
use dashboard_text::table_access::{Available, rows};

use super::model_selection::EquipmentModel;

pub const DEFAULT_MODEL_KEY: &[u8] = b"USE_DEFAULT_MODEL";

pub type Label = Result<Vec<u8>, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Country {
    tag: Vec<u8>,
    original: Result<Vec<u8>, String>,
}

pub struct Labels<'a> {
    localisation: &'a Localisation,
    countries: BTreeMap<u32, Country>,
    unavailable: Option<String>,
    dynamic: Result<&'a BTreeSet<String>, String>,
}

impl<'a> Labels<'a> {
    pub fn read(
        frame: &DecodedFrame,
        localisation: &'a Localisation,
        dynamic: Result<&'a BTreeSet<String>, String>,
    ) -> Self {
        let mut labels = Self {
            localisation,
            countries: BTreeMap::new(),
            unavailable: None,
            dynamic,
        };
        let read = || -> Result<BTreeMap<u32, Country>, String> {
            let countries = frame
                .complete::<tables::Countries>()
                .map_err(|error| error.to_string())?;
            let appearance: BTreeMap<u32, _> = match rows::<tables::CountryAppearance>(frame)
                .map_err(|error| error.to_string())?
            {
                Available::Complete(rows) | Available::Truncated { rows, .. } => {
                    rows.iter().map(|row| (row.country, row)).collect()
                }
                Available::Unavailable { reason } => {
                    return Err(format!("CountryAppearance unavailable: {reason}"));
                }
                Available::NotRecorded { mode } => {
                    return Err(format!(
                        "CountryAppearance is not recorded in a {mode} recording"
                    ));
                }
            };
            Ok(countries
                .iter()
                .map(|country| {
                    let original = appearance
                        .get(&country.id)
                        .ok_or_else(|| format!("country {} has no appearance row", country.id))
                        .and_then(|row| country_flags::original_tag(country, row))
                        .map(|tag| tag.as_bytes().to_vec());
                    (
                        country.id,
                        Country {
                            tag: country.tag.clone(),
                            original,
                        },
                    )
                })
                .collect())
        };
        match read() {
            Ok(countries) => labels.countries = countries,
            Err(reason) => labels.unavailable = Some(reason),
        }
        labels
    }

    pub(crate) fn operands(&self) -> (&BTreeMap<u32, Country>, Option<&String>) {
        (&self.countries, self.unavailable.as_ref())
    }

    fn localise(&self, key: &[u8]) -> Label {
        self.localisation
            .localise_bytes(key, &[])
            .map_err(|error| error.to_string())
    }

    pub fn default_label(&self) -> Label {
        self.localise(DEFAULT_MODEL_KEY)
    }

    pub fn archetype(&self, equipment: &EquipmentModel<'_>) -> Label {
        self.localise(equipment.archetype.as_bytes())
    }

    pub fn subunit(&self, viewing: u32, key: &str) -> Label {
        self.prefixed(viewing, key.as_bytes())
    }

    fn prefixed(&self, country: u32, key: &[u8]) -> Label {
        let tag: &[u8] = if country == 0 {
            &[]
        } else {
            &self.country(country)?.tag
        };
        self.prefixed_tag(tag, key)
    }

    fn prefixed_tag(&self, tag: &[u8], key: &[u8]) -> Label {
        let mut prefixed = tag.to_vec();
        prefixed.push(b'_');
        prefixed.extend_from_slice(key);
        let defined = std::str::from_utf8(&prefixed)
            .ok()
            .is_some_and(|text| self.localisation.get(text).is_some());
        self.localise(if defined { &prefixed } else { key })
    }

    fn country(&self, reference: u32) -> Result<&Country, String> {
        if let Some(reason) = &self.unavailable {
            return Err(format!("model labels: {reason}"));
        }
        self.countries
            .get(&reference)
            .ok_or_else(|| format!("model labels: country {reference} has no Countries row"))
    }

    pub fn display(&self, variant: &VariantDefinitionRow, equipment_name: &str) -> Label {
        let mut label = if variant.display_name.is_empty() {
            let tag = if (variant.origin_link as i32) > 0 {
                self.country(variant.origin_link)?.tag.clone()
            } else if variant.creator_link == 0 {
                Vec::new()
            } else {
                let creator = self.country(variant.creator_link)?;
                let dynamic = self.dynamic.as_ref().map_err(Clone::clone)?;
                if dynamic.contains(String::from_utf8_lossy(&creator.tag).as_ref()) {
                    creator.original.clone()?
                } else {
                    creator.tag.clone()
                }
            };
            self.prefixed_tag(&tag, equipment_name.as_bytes())?
        } else {
            variant.display_name.clone()
        };
        if variant.append_variant_name != 0 && !variant.variant_name.is_empty() {
            label.push(b' ');
            label.extend_from_slice(&variant.variant_name);
        }
        Ok(label)
    }

    pub fn variant(
        &self,
        variant: &VariantDefinitionRow,
        equipment_name: &str,
        equipment: &EquipmentModel<'_>,
    ) -> Label {
        if variant.display_name.is_empty() {
            for key in [equipment.derived_variant_name, equipment.variant_name] {
                if !key.is_empty() {
                    return self.prefixed(variant.creator_link, key);
                }
            }
        }
        self.display(variant, equipment_name)
    }
}

pub fn number_repeats(labels: &mut [Label]) {
    let mut seen: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
    let mut unknown: Option<String> = None;
    for label in labels.iter_mut() {
        match (label.as_mut(), &unknown) {
            (Err(reason), None) => unknown = Some(reason.clone()),
            (Err(_), Some(_)) => {}
            (Ok(_), Some(reason)) => {
                *label = Err(format!(
                    "its repeat number depends on an earlier unknown label ({reason})"
                ));
            }
            (Ok(text), None) => {
                let count = seen.entry(text.clone()).or_default();
                *count += 1;
                if *count > 1 {
                    text.push(b' ');
                    text.extend_from_slice(count.to_string().as_bytes());
                }
            }
        }
    }
}

pub fn second_row(default: &[u8], rows: &[&Label]) -> Result<Option<usize>, String> {
    if rows.is_empty() {
        return Ok(None);
    }
    let wraps = rows.len() >= 256;
    let mut distinct: BTreeSet<&[u8]> = BTreeSet::from([default]);
    for (index, label) in rows.iter().enumerate() {
        let label = match label {
            Ok(label) => label.as_slice(),
            Err(reason) => return Err(reason.clone()),
        };
        if label == default {
            return Ok(Some(index));
        }
        if distinct.insert(label) && wraps && (distinct.len() - 1).is_multiple_of(256) {
            return Ok(Some(index));
        }
    }
    Ok(Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryRow, rows::Recorded, rows::TableId,
        rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn label(text: &str) -> Label {
        Ok(text.as_bytes().to_vec())
    }

    fn frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [(1, "GER"), (2, "D01")]
                .into_iter()
                .map(|(id, tag)| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: tag.into(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 1,
                    original_tag: Recorded::Value(None),
                    original_tag_reference: Recorded::Value(Some(0)),
                    ..Default::default()
                },
                CountryAppearanceRow {
                    country: 2,
                    original_tag: Recorded::Value(Some(b"GER".to_vec())),
                    original_tag_reference: Recorded::Value(Some(1)),
                    ..Default::default()
                },
            ]));
        frame
    }

    fn localisation() -> Localisation {
        [
            ("USE_DEFAULT_MODEL", "Use default model"),
            ("infantry_equipment", "Infantry Equipment"),
            ("infantry_equipment_1", "Infantry Equipment I"),
            ("GER_infantry_equipment_1", "Gewehr 98"),
            ("infantry", "Infantry"),
            ("derived_name", "Derived"),
            ("GER_derived_name", "Abgeleitet"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    fn equipment(derived: &'static [u8], variant: &'static [u8]) -> EquipmentModel<'static> {
        EquipmentModel {
            archetype: "infantry_equipment",
            archetype_ordinal: 0,
            family: "infantry_equipment_1",
            model_priority: 1,
            visual_level: 1,
            model: "",
            depth: 0,
            derived_variant_name: derived,
            variant_name: variant,
        }
    }

    #[test]
    fn labels_follow_the_country_prefixed_display_language() {
        let frame = frame();
        let localisation = localisation();
        let dynamic = BTreeSet::from(["D01".to_owned()]);
        let labels = Labels::read(&frame, &localisation, Ok(&dynamic));
        assert_eq!(labels.default_label(), label("Use default model"));
        assert_eq!(
            labels.archetype(&equipment(b"", b"")),
            label("Infantry Equipment")
        );
        assert_eq!(labels.subunit(1, "infantry"), label("Infantry"));
        let mut variant = VariantDefinitionRow {
            creator_link: 1,
            ..Default::default()
        };
        assert_eq!(
            labels.display(&variant, "infantry_equipment_1"),
            label("Gewehr 98")
        );
        variant.creator_link = 2;
        assert_eq!(
            labels.display(&variant, "infantry_equipment_1"),
            label("Gewehr 98")
        );
        variant.origin_link = 2;
        assert_eq!(
            labels.display(&variant, "infantry_equipment_1"),
            label("Infantry Equipment I")
        );
        variant.display_name = b"Custom".to_vec();
        variant.variant_name = b"Mk II".to_vec();
        variant.append_variant_name = 1;
        assert_eq!(
            labels.display(&variant, "infantry_equipment_1"),
            label("Custom Mk II")
        );
        variant.display_name.clear();
        variant.creator_link = 1;
        assert_eq!(
            labels.variant(
                &variant,
                "infantry_equipment_1",
                &equipment(b"derived_name", b"x")
            ),
            label("Abgeleitet")
        );
        assert_eq!(
            labels.variant(
                &variant,
                "infantry_equipment_1",
                &equipment(b"", b"derived_name")
            ),
            label("Abgeleitet")
        );
        let unread = Labels::read(&frame, &localisation, Err("unread".into()));
        variant.origin_link = 0;
        assert!(unread.display(&variant, "infantry_equipment_1").is_err());
    }

    #[test]
    fn a_label_after_an_unknown_one_is_unknown_even_when_its_row_is_dropped() {
        let mut list = vec![label("Default"), Err("no tag".into()), label("Rifle")];
        number_repeats(&mut list);
        assert_eq!(list[0], label("Default"));
        assert!(list[2].is_err(), "{:?}", list[2]);
        let earlier: Vec<Label> = std::iter::once(label("Rifle"))
            .chain((1..255).map(|n| label(&format!("L{n}"))))
            .collect();
        let mut drained: Vec<&Label> = earlier.iter().collect();
        drained.push(&list[2]);
        assert_eq!(drained.len(), 256);
        assert!(second_row(b"Default", &drained).is_err());
    }

    #[test]
    fn repeats_are_numbered_after_the_first() {
        let mut labels = vec![
            label("Use default"),
            label("Rifle"),
            label("Rifle"),
            label("Use default"),
            label("Rifle"),
        ];
        number_repeats(&mut labels);
        assert_eq!(
            labels,
            [
                label("Use default"),
                label("Rifle"),
                label("Rifle 2"),
                label("Use default 2"),
                label("Rifle 3"),
            ]
        );
    }

    #[test]
    fn the_second_row_is_the_first_drained_unless_a_label_ranks_with_the_default() {
        let (a, b, d) = (label("A"), label("B"), label("Default"));
        assert_eq!(second_row(b"Default", &[&a, &b]), Ok(Some(0)));
        assert_eq!(second_row(b"Default", &[&a, &b, &d]), Ok(Some(2)));
        assert_eq!(second_row(b"Default", &[]), Ok(None));
        let unknown: Label = Err("no tag".into());
        assert!(second_row(b"Default", &[&a, &unknown]).is_err());
    }

    #[test]
    fn the_byte_rank_wraps_at_the_257th_distinct_label() {
        let labels: Vec<Label> = (0..300).map(|n| label(&format!("L{n}"))).collect();
        let refs: Vec<&Label> = labels.iter().collect();
        assert_eq!(second_row(b"Default", &refs), Ok(Some(255)));
        assert_eq!(second_row(b"Default", &refs[..255]), Ok(Some(0)));
    }
}
