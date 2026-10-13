use super::*;

#[test]
fn thermal_identifier_notes_reach_province_content_warnings() {
    let content = Content {
        members: Default::default(),
        impassable: Default::default(),
        neighbors: Ok(Default::default()),
        temperature: Ok(installed_content::weather::TemperatureRanges {
            ranges: Default::default(),
            notes: vec!["fixture modifier: unsupported identifier field (1 modifier)".into()],
        }),
        pass: Err("unread".into()),
        derived: Default::default(),
        units: Err("unused".into()),
        border: Err("unused".into()),
        air_log_timeout: Err("unused".into()),
    };
    assert!(content.warnings().iter().any(|warning| warning ==
        "fixture modifier: unsupported identifier field (1 modifier)"));
    assert!(
        content.warnings().iter().any(|warning| warning.contains(
            "Province temperatures cannot be derived from the recorded content (unread)"
        ))
    );
}

#[test]
fn native_multiline_indentation_keeps_gaps_and_refusals_out_of_text() {
    let mut tooltip = Tooltip::default();
    line(
        &mut tooltip,
        "location",
        0,
        Ok("Forest in Test state".into()),
    );
    line(&mut tooltip, "owner", 1, Ok("Owner\n\nController".into()));
    line(
        &mut tooltip,
        "missing input",
        1,
        Err("operand unavailable".into()),
    );
    line(&mut tooltip, "vp", 1, Ok("Victory point: 1".into()));
    assert_eq!(
        tooltip.title,
        "Forest in Test state\n    Owner\n\n    Controller\n    Victory point: 1"
    );
    assert_eq!(
        tooltip.refused["province missing input: operand unavailable"],
        1
    );
    assert!(tooltip.body.is_empty());
}
