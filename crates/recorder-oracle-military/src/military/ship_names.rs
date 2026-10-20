//! Compare serialized naming state; the retained display cache is unsaved.
use super::Report;
use installed_content_save::ShipName;
use replay_data::rows::{Recorded, ShipNamingOperands};

pub(super) fn compare(
    label: &str,
    actual: &Recorded<Option<ShipNamingOperands>>,
    expected: Option<&ShipName>,
    report: &mut Report,
) {
    report.compared("Ships.naming", 1);
    let Some(expected) = expected else {
        report.difference(
            format!("{label}.ship_name"),
            "naming operands expected",
            "missing wrapper",
        );
        return;
    };
    let Ok(Some(actual)) = actual.require("Ships.naming") else {
        report.difference(
            format!("{label}.ship_name"),
            "naming operands not recorded",
            expected,
        );
        return;
    };
    macro_rules! field {
        ($name:literal, $actual:expr, $expected:expr) => {{
            report.compared(concat!("Ships.naming.", $name), 1);
            let (actual, expected) = ($actual, $expected);
            if actual != expected {
                report.difference(format!("{label}.ship_name.{}", $name), actual, expected);
            }
        }};
    }
    field!("type", actual.kind, expected.kind);
    field!(
        "name_order",
        (actual.order != 0).then_some(actual.order),
        expected.name_order
    );
    field!(
        "is_name_ordered",
        (actual.ordered == 0).then_some(false),
        expected.is_name_ordered
    );
    let text = recorder_oracle_support::support::serialized_text(
        report,
        &format!("{label}.ship_name.override"),
        &actual.override_name,
    );
    field!(
        "override",
        (!actual.override_name.is_empty()).then_some(text),
        expected.override_name.as_deref()
    );
    field!(
        "override_set_programmatically",
        (actual.programmatic != 0).then_some(true),
        expected.override_set_programmatically
    );
    field!("equipment", actual.equipment, expected.equipment);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operands() -> ShipNamingOperands {
        ShipNamingOperands {
            kind: 1,
            order: -7,
            ordered: 0,
            override_name: b"\0tail".to_vec(),
            programmatic: 0xfe,
            equipment: Some((u64::from(u32::MAX) << 32) | 70),
        }
    }
    fn saved() -> ShipName {
        ShipName {
            kind: 1,
            name_order: Some(-7),
            is_name_ordered: Some(false),
            override_name: Some(Vec::new()),
            override_set_programmatically: Some(true),
            equipment: operands().equipment,
        }
    }
    #[test]
    fn ship_names_compare_each_field_under_omission_and_first_nul_rules() {
        let mut report = Report::default();
        compare(
            "ship",
            &Some(operands()).into(),
            Some(&saved()),
            &mut report,
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        for field in [
            "type",
            "name_order",
            "is_name_ordered",
            "override",
            "override_set_programmatically",
            "equipment",
        ] {
            assert_eq!(report.compared_tables[&format!("Ships.naming.{field}")], 1);
        }
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|line| line.contains("override"))
        );
        let actual = ShipNamingOperands {
            kind: -1,
            ordered: 1,
            ..Default::default()
        };
        let saved = ShipName {
            kind: -1,
            name_order: None,
            is_name_ordered: None,
            override_name: None,
            override_set_programmatically: None,
            equipment: None,
        };
        let mut report = Report::default();
        compare("ship", &Some(actual).into(), Some(&saved), &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
    }
    #[test]
    fn ship_names_fail_each_saved_operand_independently() {
        for field in [
            "type",
            "name_order",
            "is_name_ordered",
            "override",
            "override_set_programmatically",
            "equipment",
        ] {
            let mut expected = saved();
            match field {
                "type" => expected.kind = 2,
                "name_order" => expected.name_order = Some(7),
                "is_name_ordered" => expected.is_name_ordered = None,
                "override" => expected.override_name = None,
                "override_set_programmatically" => expected.override_set_programmatically = None,
                "equipment" => expected.equipment = Some(70),
                _ => unreachable!(),
            }
            let mut report = Report::default();
            compare(
                "ship",
                &Some(operands()).into(),
                Some(&expected),
                &mut report,
            );
            assert_eq!(
                report.differences.len(),
                1,
                "{field}: {:?}",
                report.differences
            );
            assert!(report.differences[0].contains(&format!("ship_name.{field}")));
        }
        for (field, mutate) in [
            ("name_order", 0),
            ("is_name_ordered", 1),
            ("override_set_programmatically", 2),
        ] {
            let mut actual = operands();
            let mut expected = saved();
            match mutate {
                0 => {
                    actual.order = 0;
                    expected.name_order = Some(0);
                }
                1 => {
                    actual.ordered = 2;
                    expected.is_name_ordered = Some(true);
                }
                _ => {
                    actual.programmatic = 0;
                    expected.override_set_programmatically = Some(false);
                }
            }
            let mut report = Report::default();
            compare("ship", &Some(actual).into(), Some(&expected), &mut report);
            assert_eq!(report.differences.len(), 1, "{field}");
        }
    }
    #[test]
    fn ship_names_missing_wrapper_and_refused_operands_are_differences() {
        for actual in [Recorded::NotRecorded, None.into(), Some(operands()).into()] {
            let mut report = Report::default();
            compare("ship", &actual, None, &mut report);
            assert_eq!(report.differences.len(), 1);
        }
        for actual in [Recorded::NotRecorded, None.into()] {
            let mut report = Report::default();
            compare("ship", &actual, Some(&saved()), &mut report);
            assert_eq!(report.differences.len(), 1);
        }
    }
}
