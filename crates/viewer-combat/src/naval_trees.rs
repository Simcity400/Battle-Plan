use super::naval_labels::RecordedLabels;
use content_reader_localisation::localisation::Localisation;
use replay_data::{
    naval_trees, naval_trees::SideSource, naval_trees::Trees, rows::ModifierBreakdownNode,
    rows::ModifierDefinitionRow, rows::NavalCombatShipRow, rows::NavalCombatSideDetails,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use dashboard_text::table_access::{Available, rows};
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;
use viewer_foundation::tooltip_text::TextIcons;
use viewer_foundation::tooltip_text::validate;

const ONE: i64 = 100_000;

pub(super) struct Sources<'a> {
    trees: Result<Trees<'a>, String>,
    definitions: Result<BTreeMap<u32, &'a ModifierDefinitionRow>, String>,
    labels: &'a RecordedLabels,
}

impl<'a> Sources<'a> {
    pub(super) fn read(frame: &'a DecodedFrame, labels: &'a RecordedLabels) -> io::Result<Self> {
        let layout = replay_derive::fixed_layout::validate_recording(
            frame,
            replay_derive::fixed_layout::NAVAL_TREES,
        );
        let trees = match rows::<tables::NavalModifierTrees>(frame)? {
            Available::Complete(rows) => layout.and_then(|()| {
                Trees::new(rows).ok_or_else(|| "NavalModifierTrees is out of key order".to_owned())
            }),
            Available::Truncated { missing, .. } => Err(format!(
                "NavalModifierTrees is truncated this hour ({missing} rows not reached)"
            )),
            Available::Unavailable { reason } => Err(format!(
                "the naval source trees are not available this hour (NavalModifierTrees: {reason})"
            )),
            Available::NotRecorded { mode } => Err(format!(
                "the naval source trees are not recorded in a {mode} recording"
            )),
        };
        let definitions = match rows::<tables::ModifierDefinitions>(frame)? {
            Available::Complete(rows) => Ok(rows.iter().map(|row| (row.index, row)).collect()),
            _ => Err("ModifierDefinitions is incomplete".to_owned()),
        };
        Ok(Self {
            trees,
            definitions,
            labels,
        })
    }

    #[cfg(test)]
    pub(super) fn from_parts(
        trees: Result<&'a [replay_data::rows::NavalModifierTreeRow], String>,
        definitions: &'a [ModifierDefinitionRow],
        labels: &'a RecordedLabels,
    ) -> Self {
        Self {
            trees: trees.map(|rows| Trees::new(rows).expect("rows in key order")),
            definitions: Ok(definitions.iter().map(|row| (row.index, row)).collect()),
            labels,
        }
    }

    fn trees(&self) -> Result<&Trees<'a>, String> {
        self.trees.as_ref().map_err(Clone::clone)
    }

    pub(super) fn definition(&self, key: u32) -> Result<&'a ModifierDefinitionRow, String> {
        self.definitions
            .as_ref()
            .map_err(Clone::clone)?
            .get(&key)
            .copied()
            .ok_or_else(|| format!("modifier {key:#x} has no recorded definition"))
    }

    pub(super) fn definitions(&self) -> Result<&BTreeMap<u32, &'a ModifierDefinitionRow>, String> {
        self.definitions.as_ref().map_err(Clone::clone)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the native call's side, key, prefix and output are separate operands"
    )]
    pub(super) fn side(
        &self,
        combat: u64,
        side: &NavalCombatSideDetails,
        opposing: &NavalCombatSideDetails,
        key: u32,
        prefix: &str,
        out: &mut String,
        icons: &dyn TextIcons,
        loc: &Localisation,
    ) -> Result<i64, String> {
        let trees = self.trees()?;
        let definition = self.definition(key)?;
        let mut total = 0_i64;
        for &source in naval_trees::side_sources(key) {
            let Some(owner) = naval_trees::side_source(combat, side, opposing, source) else {
                continue;
            };
            if source == SideSource::Weather && trees.row(owner).is_none() {
                continue;
            }
            let tree = trees.tree(owner, key);
            total = total.wrapping_add(tree.root());
            out.push_str(&breakdown(
                tree.nodes(),
                definition,
                ONE,
                prefix,
                self.labels,
                icons,
                loc,
                true,
            )?);
            if source != SideSource::Country {
                ensure_newline(out);
            }
        }
        Ok(total)
    }

    pub(super) fn ship(
        &self,
        row: &NavalCombatShipRow,
        key: u32,
        prefix: &str,
        out: &mut String,
        icons: &dyn TextIcons,
        loc: &Localisation,
    ) -> Result<(), String> {
        let trees = self.trees()?;
        let definition = self.definition(key)?;
        let context = &row.weapon_context;
        for source in &context.sources {
            let Some(owner) = naval_trees::ship_source(row.ship, context, source) else {
                continue;
            };
            if !super::naval_environment::breakdown_visible(
                context,
                key,
                source.scope,
                source.index,
            )
            .ok_or_else(|| {
                format!(
                    "side {} slot {} support source {}:{} selection is unavailable",
                    row.side, row.battle_slot, source.scope, source.index
                )
            })? {
                continue;
            }
            let scale =
                super::naval_environment::breakdown_scale(context, source.scope, source.index)
                    .ok_or_else(|| {
                        format!(
                            "side {} slot {} source {}:{} scale is unavailable",
                            row.side, row.battle_slot, source.scope, source.index
                        )
                    })?;
            out.push_str(&breakdown(
                trees.tree(owner, key).nodes(),
                definition,
                scale,
                prefix,
                self.labels,
                icons,
                loc,
                !matches!(source.scope, 12..=15),
            )?);
        }
        Ok(())
    }
}

fn ensure_newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Render recorded modifier trees with explicit scaling, label policy and text resources"
)]
pub(super) fn breakdown(
    nodes: &[ModifierBreakdownNode],
    definition: &ModifierDefinitionRow,
    scale: i64,
    prefix: &str,
    labels: &RecordedLabels,
    icons: &dyn TextIcons,
    loc: &Localisation,
    show_names: bool,
) -> Result<String, String> {
    if nodes.is_empty() {
        return Ok(String::new());
    }
    dashboard_text::modifier_text::breakdown(
        nodes,
        dashboard_text::modifier_text::Breakdown {
            depth: -1,
            prefix,
            suffix: "",
        },
        |node| {
            let mut text = if show_names {
                labels.text(loc, &node.name, "naval modifier source")?
            } else {
                String::new()
            };
            validate(icons, &text)
                .map_err(|reason| format!("naval modifier source holds {reason}"))?;
            if !text.is_empty() {
                text.push_str(": ");
            }
            text.push_str(
                &super::modifier_text::value_text(
                    definition,
                    super::fixed::mul(node.value, scale),
                    2,
                    false,
                    false,
                    icons,
                    loc,
                )
                .map_err(|reason| reason.to_string())?,
            );
            Ok(text)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::NavalCombatSummaryInputs, rows::NavalModifierKeyTree, rows::NavalModifierTreeRow,
        rows::NavalShipModifierSource, rows::NavalShipWeaponContext,
    };

    fn leaf(value: i64, name: &[u8]) -> Vec<ModifierBreakdownNode> {
        vec![
            ModifierBreakdownNode {
                children: 1,
                value,
                name: Vec::new(),
            },
            ModifierBreakdownNode {
                children: 0,
                value,
                name: name.to_vec(),
            },
        ]
    }

    fn row(
        owner: naval_trees::Owner,
        key: u32,
        nodes: Vec<ModifierBreakdownNode>,
    ) -> NavalModifierTreeRow {
        NavalModifierTreeRow {
            kind: owner.kind as u8,
            owner: owner.owner,
            sub: owner.sub,
            trees: vec![NavalModifierKeyTree { key, nodes }],
        }
    }

    fn sources<'a>(
        rows: &'a [NavalModifierTreeRow],
        definitions: &'a [ModifierDefinitionRow],
    ) -> Sources<'a> {
        Sources {
            trees: Ok(Trees::new(rows).unwrap()),
            definitions: Ok(definitions.iter().map(|row| (row.index, row)).collect()),
            labels: &ENGLISH,
        }
    }

    static ENGLISH: std::sync::LazyLock<RecordedLabels> = std::sync::LazyLock::new(|| {
        let mut english: Localisation = [("DOCTRINE", "Doctrine"), ("TRAIT", "Trait")]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        english.language = "english".into();
        RecordedLabels::from_localisation(&english)
    });

    fn side(country: u32, commander: u64) -> NavalCombatSideDetails {
        NavalCombatSideDetails {
            commander,
            summary: NavalCombatSummaryInputs {
                countries: vec![country],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn loc() -> Localisation {
        let mut loc = Localisation::default();
        loc.language = "english".into();
        loc
    }

    #[test]
    fn side_sources_follow_the_native_order_newlines_and_sum() {
        use naval_trees::{Kind, Owner};
        let key = 0x1b2;
        let rows = [
            row(Owner::new(Kind::Country, 7, 0), key, leaf(10_000, b"Ideas")),
            row(
                Owner::new(Kind::Commander, 66, 0),
                key,
                leaf(5_000, b"Trait"),
            ),
            row(
                Owner::new(Kind::SideTerrain, 1, 0),
                key,
                vec![ModifierBreakdownNode {
                    children: 0,
                    value: -2_000,
                    name: b"Fjords".to_vec(),
                }],
            ),
        ];
        let definitions = [ModifierDefinitionRow {
            index: key,
            precision: 2,
            ..Default::default()
        }];
        let sources = sources(&rows, &definitions);
        let mut out = String::from("Positioning\n");
        let total = sources
            .side(
                1,
                &side(7, 66),
                &side(8, 0),
                key,
                "",
                &mut out,
                &NoIcons,
                &loc(),
            )
            .unwrap();
        assert_eq!(total, 13_000);
        assert!(out.starts_with("Positioning\nIdeas: "), "{out}");
        assert!(out.contains("\nFjords: "), "{out}");
        assert!(out.ends_with('\n'), "{out}");
        assert_eq!(out.matches("Trait").count(), 1);
        let mut out = String::new();
        let total = sources
            .side(
                1,
                &side(7, 0),
                &side(8, 0),
                key,
                "",
                &mut out,
                &NoIcons,
                &loc(),
            )
            .unwrap();
        assert_eq!(total, 8_000);
        assert!(!out.contains("Trait"));
    }

    #[test]
    fn ship_sources_print_in_recorded_order() {
        use naval_trees::{Kind, Owner};
        let key = 0x5b;
        let context = NavalShipWeaponContext {
            task_force: 77,
            country: 7,
            commander: 66,
            sources: [9_u8, 0, 20, 8]
                .into_iter()
                .map(|scope| NavalShipModifierSource {
                    scope,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let ship = NavalCombatShipRow {
            ship: 41,
            live_ship: 1,
            weapon_context: context,
            ..Default::default()
        };
        let rows = [
            row(
                Owner::new(Kind::Country, 7, 0),
                key,
                leaf(10_000, b"Doctrine"),
            ),
            row(
                Owner::new(Kind::Commander, 66, 0),
                key,
                leaf(5_000, b"Trait"),
            ),
        ];
        let definitions = [ModifierDefinitionRow {
            index: key,
            precision: 2,
            ..Default::default()
        }];
        let sources = sources(&rows, &definitions);
        let mut out = String::new();
        sources
            .ship(&ship, key, "  ", &mut out, &NoIcons, &loc())
            .unwrap();
        let trait_at = out.find("Trait").unwrap();
        let doctrine_at = out.find("Doctrine").unwrap();
        assert!(trait_at < doctrine_at, "{out}");
        assert!(out.starts_with("  Trait: "), "{out}");
        let mut german: Localisation = [("DOCTRINE", "Doktrin"), ("TRAIT", "Eigenschaft")]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        german.language = "german".into();
        let mut out = String::new();
        sources
            .ship(&ship, key, "  ", &mut out, &NoIcons, &german)
            .unwrap();
        assert!(out.starts_with("  Eigenschaft: "), "{out}");
        assert!(out.contains("Doktrin: "), "{out}");
    }

    #[test]
    fn an_unrecorded_table_refuses_by_name() {
        let sources = Sources {
            trees: Err("the naval source trees are not available this hour (NavalModifierTrees: not recorded)".into()),
            definitions: Ok(BTreeMap::new()),
            labels: &ENGLISH,
        };
        let mut out = String::new();
        let error = sources
            .side(
                1,
                &side(7, 0),
                &side(8, 0),
                0x1b2,
                "",
                &mut out,
                &NoIcons,
                &loc(),
            )
            .unwrap_err();
        assert!(error.contains("NavalModifierTrees"), "{error}");
    }
}
