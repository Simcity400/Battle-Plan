//! Shared wire types and content-independent naming operands for the yard products.
pub mod at;
pub mod emit;
pub mod layout;
pub(crate) mod management;
mod naming;
pub(crate) mod project;
pub mod read;
pub mod segment;
pub mod served;
mod ship_names;
use dashboard_base::{
    catalogue::{self, Catalogue, View},
    products::check_reply_budget,
};
pub use dashboard_keyed::query::params;
use dashboard_keyed::countries::Countries;
use dashboard_text::{
    scripted_text,
    variant_names::{self, RawVariant, Tags, Texts},
};
pub use naming::*;
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{self, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, TS)]
pub struct YardChoice {
    pub id: String,
    pub name: String,
}

/// The types both views' replies declare: the view at an hour.
pub fn types() -> Vec<String> {
    vec![
        at::YardsAtParams::decl(),
        YardChoice::decl(),
        at::YardShip::decl(),
        at::YardQueued::decl(),
        at::YardHolder::decl(),
        at::YardsAtReply::decl(),
    ]
}

/// The design names' tables: definitions, country tags and lexical tokens.
fn design_naming(
    frame: &DecodedFrame,
) -> Result<(&[rows::VariantDefinitionRow], Tags<'_>, &[rows::LexicalTokenRow]), String> {
    let variants = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|why| why.to_string())?;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|why| why.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let lexical = frame
        .complete::<tables::LexicalTokens>()
        .unwrap_or_default();
    let tags = Tags::new(
        countries,
        links,
        frame.complete::<tables::CountryAppearance>().ok(),
    );
    Ok((variants, tags, lexical))
}

fn design_operand(
    definition: &rows::VariantDefinitionRow,
    tags: &Tags<'_>,
    lexical: &[rows::LexicalTokenRow],
) -> Operand {
    // Dates are separate channels, never an interned token per hour.
    Operand::Design(variant_names::raw_variant(definition, 0, tags, lexical))
}

pub fn find<T, K: Ord>(rows: Option<&[T]>, key: K, by: impl Fn(&T) -> K) -> Option<&T> {
    let rows = rows?;
    rows.binary_search_by_key(&key, by).ok().map(|at| &rows[at])
}

/// Each source table as an hour holds it, which dates the hour's supplying reads.
#[derive(replay_spool::Spooled)]
pub(crate) struct Sources {
    hour: i32,
    tables: Vec<Source>,
}

#[derive(replay_spool::Spooled)]
pub(crate) struct Source {
    table: TableId,
    /// Why it does not hold its own complete rows: the refusal it publishes then.
    missing: Option<String>,
    /// Whether its slot was read at the hour.
    read: bool,
    /// The hour of the acquisition its view holds.
    sampled: Option<i32>,
    /// Whether its reads are dated.
    dated: bool,
}

impl Sources {
    pub(crate) fn read(format: FormatVersion, frame: &DecodedFrame, sources: &[TableId]) -> Self {
        Self {
            hour: frame.hour,
            tables: sources
                .iter()
                .map(|&table| Source {
                    table,
                    missing: (!known(frame, table)).then(|| {
                        let reason = dashboard_base::products::table_missing_reason(frame, table)
                            .unwrap_or_else(|| "Table not recorded".into());
                        format!("{table:?}: {reason}")
                    }),
                    read: dashboard_text::metadata::slot_read(
                        format,
                        &frame.metrics,
                        table,
                        frame.hour,
                    ),
                    sampled: frame.tables[table.index()].sampled_at(),
                    dated: dated(table, format),
                })
                .collect(),
        }
    }

    pub(crate) fn heap(&self) -> usize {
        self.tables.capacity() * size_of::<Source>()
            + self
                .tables
                .iter()
                .filter_map(|source| source.missing.as_ref())
                .map(String::capacity)
                .sum::<usize>()
    }
}

/// Whether `table` holds its own complete rows at the hour.
fn known(frame: &DecodedFrame, table: TableId) -> bool {
    matches!(&*frame.tables[table.index()], replay_frame::frame::TableView::Complete(rows) if rows.table() == table)
}

/// Whether a known source carries the date of its supplying read.
fn dated(table: TableId, format: FormatVersion) -> bool {
    !matches!(
        replay_data::cadence::cadence_in(table, format),
        replay_data::cadence::Cadence::Hourly
    )
}

pub fn owners(frame: &DecodedFrame, countries: &Countries) -> Vec<String> {
    let mut owners: Vec<_> = frame
        .complete::<tables::Countries>()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| countries.slotted(row.canonical_slot))
        .collect();
    owners.sort();
    owners.dedup();
    owners
}

pub fn fields(tables: &[TableId]) -> io::Result<Vec<TableFields>> {
    tables
        .iter()
        .map(|&table| {
            let names: &[&str] = match table {
                TableId::Countries => &["canonical_slot", "tag"],
                TableId::CountryAppearance => {
                    &["name_tag", "original_tag", "original_tag_reference"]
                }
                TableId::ReservedCountryNaming => {
                    &["tag", "original_tag", "original_tag_reference"]
                }
                TableId::Ships => &["task_force", "strength", "class_id", "name", "naming"],
                TableId::ShipModels => &["counter", "variant", "subunit_index"],
                TableId::NavalRepairTaskForces => &[
                    "repair_mode",
                    "repair_split",
                    "repair_target",
                    "merge_after_repair",
                    "refit_variant",
                    "refit_after_repair",
                ],
                TableId::TaskForces => &["roster_owner"],
                TableId::TaskForceCounterInputs => &["name"],
                TableId::CountryFactories => &["naval_total_raw"],
                TableId::NavalBases => &["country"],
                TableId::ProductionLines => &["cost_raw", "active_factories", "variant"],
                TableId::Provinces => &["state"],
                TableId::States => &["owner", "custom_name", "owner_name"],
                _ => return Ok(TableFields::all(table)),
            };
            TableFields::named(table, names)
        })
        .collect()
}

pub fn checked<T: Serialize>(reply: &T) -> io::Result<serde_json::Value> {
    check_reply_budget(reply)?;
    serde_json::to_value(reply).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_mod_names_design_suffixes_and_policy_keys_follow_layered_localisation() {
        let (_, base) = catalogue::tests::folder_catalogue(&[(
            "localisation/base_l_english.yml",
            "\u{feff}l_english:\n mod_port:0 \"Base port\"\n mod_class:0 \"Base class\"\n mod_design:0 \"Base design\"\n",
        )]);
        let (_, overlay) = catalogue::tests::folder_catalogue(&[(
            "localisation/replace/overlay_l_english.yml",
            "\u{feff}l_english:\n mod_port:0 \"Overlay port\"\n mod_class:0 \"Overlay class\"\n mod_design:0 \"Overlay design\"\n NAVY_DOCKYARD_REPAIR_POLICY_-3:0 \"Overlay priority\"\n",
        )]);
        let layers = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[][..]),
            (&overlay.0, &[][..]),
        ])
        .unwrap();
        let cat = Arc::new(catalogue::tests::layers_catalogue(layers));
        let port = Operand::Port(PortName {
            province: 9,
            override_name: Ok(b"mod_port".to_vec()),
            owner_tag: Err("unused owner".into()),
            state: Err("unused state".into()),
        });
        assert_eq!(resolve(&port, Some(&cat)).unwrap().name, "Overlay port");
        let class = Operand::Class {
            token: 712,
            spelling: Ok("mod_class".into()),
            index: None,
        };
        assert_eq!(resolve(&class, Some(&cat)).unwrap().id, "mod_class");
        assert_eq!(resolve(&class, Some(&cat)).unwrap().name, "Overlay class");
        let design = Operand::Design(RawVariant {
            id: u64::MAX,
            token: "mod_design".into(),
            suffix: "Mark synthetic".into(),
            append: true,
            ..Default::default()
        });
        assert_eq!(
            resolve(&design, Some(&cat)).unwrap().name,
            "Overlay design Mark synthetic"
        );
        assert_eq!(
            resolve(
                &Operand::Policy {
                    port: true,
                    value: -3
                },
                Some(&cat)
            )
            .unwrap()
            .name,
            "Overlay priority"
        );
        let hidden = installed_content_core::ResolvedLayers::from_folders(&[
            (&base.0, &[][..]),
            (&overlay.0, &["localisation"][..]),
        ])
        .unwrap();
        let hidden = Arc::new(catalogue::tests::layers_catalogue(hidden));
        assert_eq!(
            resolve(&class, Some(&hidden)).unwrap().name,
            "Overlay class"
        );
        assert!(
            resolve(
                &Operand::Class {
                    token: 900,
                    spelling: Ok("unrecorded_class".into()),
                    index: None
                },
                Some(&hidden)
            )
            .is_err()
        );
        assert!(resolve(&design, None).is_err());
    }

    #[test]
    fn port_names_refuse_an_unknown_or_unshowable_primary_name_instead_of_the_state_name() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (cat, _folder) = catalogue::tests::folder_catalogue(&[(
            "localisation/ports_l_english.yml",
            "\u{feff}l_english:\n spaced_port:0 \"Spaced ¤ port\"\n empty_port:0 \"\"\n VICTORY_POINTS_9:0 \"Victory port\"\n VICTORY_POINTS_10:0 \"Spaced ¤ victory\"\n",
        )]);
        let cat = Arc::new(cat);
        let name = |province: u32, override_name: Result<&str, &str>, owner_tag: Result<&str, &str>| {
            let port = Operand::Port(PortName {
                province,
                override_name: override_name
                    .map(|name| name.as_bytes().to_vec())
                    .map_err(str::to_owned),
                owner_tag: owner_tag.map(str::to_owned).map_err(str::to_owned),
                state: Ok(StateName {
                    id: 3,
                    custom: b"Harbour state".to_vec(),
                    owner: Vec::new(),
                }),
            });
            resolve(&port, Some(&cat)).map(|choice| choice.name)
        };
        // Only a genuinely absent primary name falls back to the state's.
        assert_eq!(name(11, Ok(""), Ok("SYN")).unwrap(), "Harbour state");
        assert_eq!(name(9, Ok("empty_port"), Ok("SYN")).unwrap(), "Victory port");
        assert_eq!(name(9, Ok("Literal port"), Ok("SYN")).unwrap(), "Literal port");
        for spaced in [
            name(9, Ok("spaced_port"), Ok("SYN")),
            name(10, Ok(""), Ok("SYN")),
        ] {
            assert!(spaced.unwrap_err().contains("plain text cannot show"));
        }
        assert_eq!(
            name(9, Err("Province names unavailable"), Ok("SYN")).unwrap_err(),
            "Province names unavailable"
        );
        assert_eq!(
            name(9, Ok(""), Err("Owner unrecorded")).unwrap_err(),
            "Owner unrecorded"
        );
    }
}
