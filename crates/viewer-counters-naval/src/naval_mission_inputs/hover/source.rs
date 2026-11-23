use replay_data::rows::{ModifierBreakdownNode, NavalMissionLedgerRow};
use super::{Fact, Hour, Localisation, ModifierDefinitionRow, Op, Prepared, data, keys, tables, tooltip_text};
use viewer_runtime::recorded_labels::RecordedLabels;

pub fn prepare_ledger(
    hour: &Hour<'_>,
    ledger: &NavalMissionLedgerRow,
    out: &mut Prepared,
) -> Fact<()> {
    let Some(&first) = ledger.task_forces.first() else {
        return Ok(());
    };
    let owner = if let Some(input) = hour.inputs.get(&first) {
        input.mission_owner
    } else {
        data::<tables::NavalPatrolMissionInputs>(hour.frame)?
            .iter()
            .find(|row| row.task_force == first)
            .and_then(|row| row.mission_owner)
            .ok_or("first escort-ledger registration mission owner is unavailable")?
    };
    prepare(hour, owner, out)
}

pub fn prepare(hour: &Hour<'_>, mission_owner: u32, out: &mut Prepared) -> Fact<()> {
    if hour.country_value(mission_owner, keys::CONVOY_ESCORT_EFFICIENCY)? == 0 {
        return Ok(());
    }
    let country = hour.country(mission_owner)?;
    let nodes = country
        .escort_tree
        .require("NavalMissionCountries.escort_tree")?
        .as_ref()
        .ok_or("escort modifier source tree was refused at capture")?;
    let definitions = hour.definitions.as_ref().map_err(Clone::clone)?;
    let definition = definitions
        .get(&keys::CONVOY_ESCORT_EFFICIENCY)
        .ok_or("escort modifier definition is unavailable")?;
    out.0
        .push(Op::ModifierTree((**definition).clone(), nodes.clone()));
    Ok(())
}

pub fn render(
    definition: &ModifierDefinitionRow,
    nodes: &[ModifierBreakdownNode],
    loc: &Localisation,
    recorded: &str,
    labels: Option<&RecordedLabels>,
) -> Fact<String> {
    let text = if nodes.is_empty() {
        String::new()
    } else {
        dashboard_text::modifier_text::breakdown(
            nodes,
            dashboard_text::modifier_text::Breakdown {
                depth: -1,
                prefix: "",
                suffix: "",
            },
            |node| {
                let mut text = if let Some(labels) = labels {
                    labels.text(loc, &node.name, "escort modifier source")?
                } else {
                    dashboard_text::scripted_text::stored_text(
                        loc,
                        recorded,
                        &node.name,
                        "escort modifier source",
                    )?
                };
                if !text.is_empty() {
                    text.push_str(": ");
                }
                text.push_str(&dashboard_text::modifier_text::value_text(
                    definition,
                    node.value,
                    2,
                    false,
                    false,
                    |key, _| loc.localise(key).map_err(|e| e.to_string()),
                )?);
                Ok(text)
            },
        )?
    };
    let mut out = tooltip_text::localise(loc, "MODIFIED_BY", &[])?;
    out.push('\n');
    out.push_str(&text);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use replay_data::rows::ModifierBreakdownNode;
    use super::{Localisation, ModifierDefinitionRow, RecordedLabels, render};
    #[test]
    fn sources_use_preorder_leaf_labels_and_metadata_in_display_language() {
        let mut recorded: Localisation = [("IDEA".into(), "Idea".into())].into_iter().collect();
        recorded.language = "english".into();
        let mut display: Localisation = [
            ("IDEA".into(), "Id\u{e9}e".into()),
            ("MODIFIED_BY".into(), "Sources:".into()),
        ]
        .into_iter()
        .collect();
        display.language = "french".into();
        let labels = RecordedLabels::from_localisation(&recorded);
        let definition = ModifierDefinitionRow {
            flags: dashboard_text::modifier_text::PERCENT_TIMES_HUNDRED
                | dashboard_text::modifier_text::POSITIVE_IS_GOOD,
            precision: 2,
            ..Default::default()
        };
        let nodes = [
            ModifierBreakdownNode {
                children: 1,
                value: 25_000,
                name: Vec::new(),
            },
            ModifierBreakdownNode {
                children: 0,
                value: 25_000,
                name: b"Idea".to_vec(),
            },
        ];
        let text = render(&definition, &nodes, &display, "english", Some(&labels)).unwrap();
        assert!(text.starts_with("Sources:\n"));
        assert!(text.contains("Id\u{e9}e: "));
        assert!(text.contains("+25.00%"));
        assert_eq!(
            render(&definition, &[], &display, "english", None).unwrap(),
            "Sources:\n"
        );
    }
}
