use super::rules::*;
use content_reader_localisation::localisation::Localisation;
fn loc() -> Localisation {
    [
        ("raid_inst_progress_desc", "$PHASE$ $PROGRESS|0%$"),
        ("tooltip_raid_result", "$RAID_DESC$ $OUTCOME_TYPE$"),
        ("tooltip_launch_button_right_click", "unit $NAME$"),
        ("tooltip_raid_warning_description", "$COUNTRY$ $RAID$"),
        (
            "tooltip_raid_warning_success_chance_header",
            "chance $VALUE|1%$",
        ),
        (
            "TOOLTIP_POTENTIAL_TARGET_ENTRY",
            "target $TARGET_NAME$ $CONTROLLER_FLAG$",
        ),
        (
            "TOOLTIP_POTENTIAL_TARGET_ENTRY_NOT_ALLOWED",
            "denied $TARGET_NAME$ $CONTROLLER_FLAG$",
        ),
        (
            "TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC",
            "unknown $CURRENT|0$/$REQUIRED|0$",
        ),
        (
            "TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC",
            "project $PROJECT_NAME$",
        ),
        ("TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL", "intel $CURRENT|0$"),
        (
            "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_ALLOWED",
            "allowed $RAID$",
        ),
        (
            "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_NOT_ALLOWED",
            "unavailable $RAID$",
        ),
        (
            "raid_type_rescue_captured_general_captive_line",
            "captive $NAME$",
        ),
        ("TOOLTIP_POTENTIAL_TARGET_RAID_DESC", "description $TEXT$"),
        ("RAID_RANGE_FACTOR_TOOLTIP", "range $RANGE_FACTOR|0%$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

fn raid_input(phase: i32) -> Raid {
    Raid {
        ended: phase == 5,
        phase,
        description: Ok("raid description".into()),
        outcome: 3,
        progress: Ok(None),
        gate: (Ok(true), vec![("test".into(), Ok("reasons".into()))]),
        assigned_unit: Ok(None),
        owner: Ok("owner".into()),
        success: Ok(75_500),
        counterplay: Ok(Vec::new()),
    }
}

#[test]
fn own_phases_progress_and_diagnostic_return_select_the_lines() {
    for phase in 0..5 {
        let out = raid(&loc(), raid_input(phase), false);
        assert!(
            out.title
                .starts_with("raid description\n\nRAID_INST_PHASE_")
        );
        assert_eq!(out.title.contains("reasons"), phase == 1);
        assert!(!out.title.contains("tooltip_launch_button_disabled"));
        assert_eq!(out.body, "tooltip_raid_view_details_prompt");
    }
    let mut input = raid_input(2);
    input.progress = Ok(Some(49_999));
    assert!(
        raid(&loc(), input, false)
            .title
            .contains("RAID_INST_PHASE_PREPARING 49%")
    );
    let mut input = raid_input(3);
    input.gate = (Ok(false), vec![("test".into(), Ok("first\nsecond".into()))]);
    input.assigned_unit = Ok(Some(Ok("named unit".into())));
    let out = raid(&loc(), input, false);
    assert!(
        out.title
            .ends_with("\n\ntooltip_launch_button_disabled\nfirst\nsecond")
    );
    assert!(out.body.ends_with("\nunit named unit"));
}

#[test]
fn ended_own_and_incoming_share_every_outcome_and_ignore_active_facts() {
    let keys = [
        (1, "FAILURE"),
        (2, "LIMITED_SUCCESS"),
        (3, "SUCCESS"),
        (4, "CRITICAL_SUCCESS"),
        (5, "CANCELED"),
    ];
    for (outcome, suffix) in keys {
        for incoming in [false, true] {
            let mut input = raid_input(5);
            input.outcome = outcome;
            input.success = Err("must not read".into());
            input.progress = Err("must not read".into());
            let out = raid(&loc(), input, incoming);
            assert_eq!(
                out.title,
                format!("raid description RAID_INST_OUTCOME_{suffix}")
            );
            assert_eq!(
                out.body,
                "tooltip_raid_view_details_prompt\ntooltip_raid_close_report_prompt"
            );
            assert!(out.refused.is_empty());
        }
    }
    for incoming in [false, true] {
        let mut input = raid_input(5);
        input.outcome = 0;
        input.success = Err("must not read".into());
        input.progress = Err("must not read".into());
        let out = raid(&loc(), input, incoming);
        assert_eq!(out.title, "raid description raid outcome was not recorded");
        assert!(!out.title.contains("RAID_INST_OUTCOME_NONE"));
        assert!(
            out.refused
                .contains_key("tooltip_raid_result: raid outcome was not recorded")
        );
    }
}

#[test]
fn incoming_orders_warning_success_and_optional_counterplay() {
    let mut input = raid_input(4);
    input.counterplay = Ok(vec![Ok("first".into()), Ok("second".into())]);
    let out = raid(&loc(), input, true);
    assert_eq!(
        out.title,
        "tooltip_raid_warning_header\nowner raid description\n\nchance 75.5%\n\ntooltip_raid_warning_counterplay_header\nfirst\nsecond"
    );
    assert!(out.body.is_empty());
    assert!(
        !raid(&loc(), raid_input(4), true)
            .title
            .contains("counterplay")
    );
}

fn kind(allowed: bool) -> Type {
    Type {
        name: Ok("type".into()),
        allowed: Ok(allowed),
        captive: Ok(None),
        description: Ok(Some("detail".into())),
        denial: vec![("test".into(), Ok("reason".into()))],
        launch_condition: Ok(Some(vec![("test".into(), Ok("trigger".into()))])),
        range: 0,
        once: false,
    }
}

fn target(facility: Facility) -> Target {
    Target {
        name: Ok("place".into()),
        flag: Ok("flag".into()),
        allowed: Ok(true),
        facility: Ok(facility),
        detection: Ok(Some(1_200_000)),
        types: vec![kind(true)],
    }
}

#[test]
fn facility_unknown_project_idle_and_detection_branches_are_distinct() {
    let cases = [
        (Facility::None, None, true),
        (
            Facility::Unknown {
                current: 1_200_000,
                required: 2_500_000,
            },
            Some("unknown 12/25"),
            false,
        ),
        (
            Facility::Project(Ok("invention".into())),
            Some("project invention"),
            true,
        ),
        (
            Facility::Idle,
            Some("TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC"),
            true,
        ),
    ];
    for (facility, line, intel) in cases {
        let out = offers(&loc(), vec![target(facility)], Ok(false));
        if let Some(line) = line {
            assert!(out.title.contains(line));
        }
        assert_eq!(out.title.contains("intel 12"), intel);
    }
    let mut input = target(Facility::None);
    input.detection = Ok(None);
    assert!(
        !offers(&loc(), vec![input], Ok(false))
            .title
            .contains("intel")
    );
}

#[test]
fn compact_offers_suppress_description_and_launch_condition_only() {
    for several_targets in [false, true] {
        let mut first = target(Facility::None);
        first.types[0].captive = Ok(Some("leader".into()));
        first.types[0].range = 150_000;
        first.types[0].once = true;
        let mut targets = vec![first];
        if several_targets {
            targets.push(target(Facility::None));
        } else {
            targets[0].types.push(kind(false));
        }
        let out = offers(&loc(), targets, Ok(false));
        assert!(!out.title.contains("description detail"));
        assert!(!out.title.contains("LAUNCH_CONDITION"));
        assert!(out.title.contains("captive leader"));
        assert!(out.title.contains("range 150%"));
        assert!(out.title.contains("tooltip_raid_only_launch_once"));
        assert_eq!(
            out.title
                .contains("TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS"),
            !several_targets
        );
        assert_eq!(out.title.contains("reason"), !several_targets);
    }
}

#[test]
fn sole_offer_allowed_denied_description_trigger_range_once_and_quick_start() {
    let out = offers(&loc(), vec![target(Facility::None)], Ok(true));
    assert!(out.title.contains(
        "allowed type\ndescription detail\n\nTOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION\ntrigger"
    ));
    assert!(out.title.ends_with("\n\nRAID_SHIFT_CLICK_QUICK_START"));
    assert!(!out.title.contains("range"));
    let mut input = target(Facility::None);
    input.allowed = Ok(false);
    input.types = vec![kind(false)];
    input.types[0].description = Ok(None);
    input.types[0].range = -1;
    let out = offers(&loc(), vec![input], Ok(false));
    assert!(out.title.starts_with("denied place flag"));
    assert!(out.title.ends_with("unavailable type\nreason"));
    assert!(!out.title.contains("description"));
    assert!(!out.title.contains("LAUNCH_CONDITION"));
    assert!(offers(&loc(), Vec::new(), Ok(false)).title.is_empty());
}

#[test]
fn unavailable_line_inputs_are_counted_and_do_not_fabricate_values() {
    let mut input = raid_input(4);
    input.success = Err("missing operand".into());
    input.counterplay = Err("missing operand".into());
    let out = raid(&loc(), input, true);
    assert_eq!(out.refused.values().sum::<usize>(), 2);
    assert!(
        out.refused
            .contains_key("tooltip_raid_warning_success_chance_header: missing operand")
    );
    assert!(out.title.contains("owner raid description"));
    assert!(!out.title.contains("chance"));
}

#[test]
fn assembling_diagnostics_survive_unknown_final_gate_and_prepared_gate_stays_unknown() {
    for phase in [1, 3] {
        let mut input = raid_input(phase);
        input.gate = (
            Err("trigger input absent".into()),
            vec![
                ("equipment".into(), Ok("equipment shortage".into())),
                ("raid trigger".into(), Err("trigger input absent".into())),
            ],
        );
        let out = raid(&loc(), input, false);
        assert_eq!(out.title.contains("equipment shortage"), phase == 1);
        assert_eq!(out.refused.values().sum::<usize>(), 1);
        assert!(!out.title.contains("tooltip_launch_button_disabled"));
    }
}

#[test]
fn unsupported_phase_and_outcome_codes_are_shown_visibly_with_raw_values() {
    let mut ended = raid_input(5);
    ended.outcome = 99;
    let out = raid(&loc(), ended, false);
    assert_eq!(out.title, "raid description Unsupported outcome (99)");
    assert!(
        out.refused
            .contains_key("tooltip_raid_result: unsupported raid outcome (99)"),
        "expected unsupported outcome refusal, got {:?}",
        out.refused
    );
    assert!(!out.title.contains("RAID_INST_OUTCOME_NONE"));

    let mut active = raid_input(42);
    active.ended = false;
    let out = raid(&loc(), active, false);
    assert_eq!(out.title, "raid description\n\nUnsupported phase (42)");
    assert!(
        out.refused
            .contains_key("raid_inst_progress_desc: unsupported raid phase (42)"),
        "expected unsupported phase refusal, got {:?}",
        out.refused
    );
    assert!(!out.title.contains("RAID_INST_PHASE_NONE"));
}
