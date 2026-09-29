//! Line order and conditions, apart from content and recorded-input joins.
use super::Tooltip;
use crate::combat::text::localise;
use installed_content::localisation::Argument as Arg;
use installed_content::localisation::Localisation;

pub type Text = Result<String, String>;
pub type Known<T> = Result<T, String>;
pub type Lines = Vec<(String, Text)>;

fn lines(out: &mut Tooltip, gap: usize, lines: Lines) {
    for (i, (name, text)) in lines.into_iter().enumerate() {
        out.line(false, if i == 0 { gap } else { 1 }, &name, text);
    }
}

pub struct Raid {
    pub ended: bool,
    pub phase: i32,
    pub description: Text,
    pub outcome: i32,
    pub progress: Known<Option<i64>>,
    /// Diagnostic-mode CanExecute's return, and its accumulated reasons.
    /// `140FD6550` returns the final trigger step even if unit/pool failed.
    pub gate: (Known<bool>, Lines),
    pub assigned_unit: Known<Option<Text>>,
    pub owner: Text,
    pub success: Known<i64>,
    pub counterplay: Known<Vec<Text>>,
}

const PHASES: [&str; 6] = [
    "RAID_INST_PHASE_NONE",
    "RAID_INST_PHASE_ASSEMBLING",
    "RAID_INST_PHASE_PREPARING",
    "RAID_INST_PHASE_PREPARED",
    "RAID_INST_PHASE_IN_PROGRESS",
    "RAID_INST_PHASE_ENDED",
];
const OUTCOMES: [&str; 6] = [
    "RAID_INST_OUTCOME_NONE",
    "RAID_INST_OUTCOME_FAILURE",
    "RAID_INST_OUTCOME_LIMITED_SUCCESS",
    "RAID_INST_OUTCOME_SUCCESS",
    "RAID_INST_OUTCOME_CRITICAL_SUCCESS",
    "RAID_INST_OUTCOME_CANCELED",
];

fn enum_key(keys: &[&'static str], value: i32) -> &'static str {
    usize::try_from(value)
        .ok()
        .and_then(|i| keys.get(i))
        .copied()
        .unwrap_or(keys[0])
}

pub fn raid(loc: &Localisation, input: Raid, incoming: bool) -> Tooltip {
    let mut out = Tooltip::default();
    let plain = |key| localise(loc, key, &[]);
    if input.ended {
        // `141613BE0`: same result for own and incoming; description here
        // uses plain location (`140FD60B0`), unlike active descriptions.
        out.line(
            false,
            0,
            "tooltip_raid_result",
            (|| {
                localise(
                    loc,
                    "tooltip_raid_result",
                    &[
                        ("RAID_DESC", Arg::Text(&input.description?)),
                        (
                            "OUTCOME_TYPE",
                            Arg::Text(&plain(enum_key(&OUTCOMES, input.outcome))?),
                        ),
                    ],
                )
            })(),
        );
        out.line(
            true,
            0,
            "tooltip_raid_view_details_prompt",
            plain("tooltip_raid_view_details_prompt"),
        );
        out.line(
            true,
            1,
            "tooltip_raid_close_report_prompt",
            plain("tooltip_raid_close_report_prompt"),
        );
        return out;
    }
    if incoming {
        // `141617380`, header, description, success, optional counterplay.
        out.line(
            false,
            0,
            "tooltip_raid_warning_header",
            plain("tooltip_raid_warning_header"),
        );
        out.line(
            false,
            1,
            "tooltip_raid_warning_description",
            (|| {
                localise(
                    loc,
                    "tooltip_raid_warning_description",
                    &[
                        ("COUNTRY", Arg::Text(&input.owner?)),
                        ("RAID", Arg::Text(&input.description?)),
                    ],
                )
            })(),
        );
        out.line(
            false,
            2,
            "tooltip_raid_warning_success_chance_header",
            input.success.and_then(|chance| {
                localise(
                    loc,
                    "tooltip_raid_warning_success_chance_header",
                    &[("VALUE", Arg::Fixed5(chance))],
                )
            }),
        );
        match input.counterplay {
            Ok(lines) if !lines.is_empty() => {
                out.line(
                    false,
                    2,
                    "tooltip_raid_warning_counterplay_header",
                    plain("tooltip_raid_warning_counterplay_header"),
                );
                for line in lines {
                    out.line(false, 1, "tooltip_raid_warning_counterplay", line);
                }
            }
            Ok(_) => {}
            Err(reason) => out.refuse("tooltip_raid_warning_counterplay", reason),
        }
        return out;
    }
    // `141E72380` puts prompts in CToolTip+0x20, main text in +0x00.
    out.line(
        true,
        0,
        "tooltip_raid_view_details_prompt",
        plain("tooltip_raid_view_details_prompt"),
    );
    out.line(false, 1, "raid_inst_item_desc", input.description);
    out.line(
        false,
        2,
        "raid_inst_progress_desc",
        input.progress.and_then(|progress| {
            let phase = plain(enum_key(&PHASES, input.phase))?;
            match progress {
                Some(value) => localise(
                    loc,
                    "raid_inst_progress_desc",
                    &[
                        ("PHASE", Arg::Text(&phase)),
                        ("PROGRESS", Arg::Fixed5(value)),
                    ],
                ),
                None => Ok(phase),
            }
        }),
    );
    // `141612820`: prepare reasons unconditionally in phase 1; execute
    // reasons only when the diagnostic-path return is false in phase 3.
    if input.phase == 1 || input.phase == 3 {
        match input.gate {
            (allowed, reasons) if input.phase == 1 || allowed == Ok(false) => {
                if input.phase == 3 {
                    out.line(
                        false,
                        2,
                        "tooltip_launch_button_disabled",
                        plain("tooltip_launch_button_disabled"),
                    );
                }
                lines(&mut out, if input.phase == 3 { 1 } else { 2 }, reasons);
            }
            (Ok(_), _) => {}
            (Err(reason), _) => out.refuse("raid_launch_reasons", reason),
        }
    }
    match input.assigned_unit {
        Ok(Some(name)) => out.line(
            true,
            1,
            "tooltip_launch_button_right_click",
            name.and_then(|name| {
                localise(
                    loc,
                    "tooltip_launch_button_right_click",
                    &[("NAME", Arg::Text(&name))],
                )
            }),
        ),
        Ok(None) => {}
        Err(reason) => out.refuse("tooltip_launch_button_right_click", reason),
    }
    out
}

pub struct Target {
    pub name: Text,
    pub flag: Text,
    pub allowed: Known<bool>,
    pub facility: Known<Facility>,
    pub detection: Known<Option<i64>>,
    pub types: Vec<Type>,
}
pub enum Facility {
    None,
    Unknown { current: i64, required: i64 },
    Project(Text),
    Idle,
}
pub struct Type {
    pub name: Text,
    pub allowed: Known<bool>,
    pub captive: Known<Option<String>>,
    pub description: Known<Option<String>>,
    pub denial: Lines,
    pub launch_condition: Known<Option<Lines>>,
    pub range: i64,
    pub once: bool,
}

/// `141615C50` -> `1416151F0` -> `141612FC0`. Only the candidate list is
/// cached; other facts are the shown hour's. Compact means anything except
/// one target with exactly one type; it suppresses description/launch text.
pub fn offers(loc: &Localisation, targets: Vec<Target>, quick: Known<bool>) -> Tooltip {
    let compact = targets.len() != 1 || targets[0].types.len() != 1;
    let mut out = Tooltip::default();
    let plain = |key| localise(loc, key, &[]);
    for target in targets {
        let header = "TOOLTIP_POTENTIAL_TARGET_ENTRY";
        out.line(
            false,
            2,
            header,
            (|| {
                let key = if target.allowed? {
                    header
                } else {
                    "TOOLTIP_POTENTIAL_TARGET_ENTRY_NOT_ALLOWED"
                };
                localise(
                    loc,
                    key,
                    &[
                        ("TARGET_NAME", Arg::Text(&target.name?)),
                        ("CONTROLLER_FLAG", Arg::Text(&target.flag?)),
                    ],
                )
            })(),
        );
        let unknown = matches!(target.facility, Ok(Facility::Unknown { .. }));
        match target.facility {
            Ok(Facility::None) => {}
            Ok(Facility::Unknown { current, required }) => out.line(
                false,
                1,
                "TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC",
                localise(
                    loc,
                    "TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC",
                    &[
                        ("CURRENT", Arg::Fixed5(current)),
                        ("REQUIRED", Arg::Fixed5(required)),
                    ],
                ),
            ),
            Ok(Facility::Project(name)) => out.line(
                false,
                1,
                "TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC",
                name.and_then(|name| {
                    localise(
                        loc,
                        "TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC",
                        &[("PROJECT_NAME", Arg::Text(&name))],
                    )
                }),
            ),
            Ok(Facility::Idle) => out.line(
                false,
                1,
                "TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC",
                plain("TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC"),
            ),
            Err(reason) => out.refuse("TOOLTIP_TARGET_FACILITY_PROJECT", reason),
        }
        if !unknown {
            match target.detection {
                Ok(Some(current)) => out.line(
                    false,
                    1,
                    "TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL",
                    localise(
                        loc,
                        "TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL",
                        &[("CURRENT", Arg::Fixed5(current))],
                    ),
                ),
                Ok(None) => {}
                Err(reason) => out.refuse("TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL", reason),
            }
        }
        if target.types.len() > 1 {
            out.line(
                false,
                2,
                "TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS",
                plain("TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS"),
            );
        }
        for kind in target.types {
            out.line(
                false,
                2,
                "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER",
                (|| {
                    let key = if kind.allowed.clone()? {
                        "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_ALLOWED"
                    } else {
                        "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_NOT_ALLOWED"
                    };
                    localise(loc, key, &[("RAID", Arg::Text(&kind.name?))])
                })(),
            );
            match kind.captive {
                Ok(Some(name)) => out.line(
                    false,
                    1,
                    "raid_type_rescue_captured_general_captive_line",
                    localise(
                        loc,
                        "raid_type_rescue_captured_general_captive_line",
                        &[("NAME", Arg::Text(&name))],
                    ),
                ),
                Ok(None) => {}
                Err(reason) => out.refuse("raid_type_rescue_captured_general_captive_line", reason),
            }
            if !compact {
                match kind.description {
                    Ok(Some(desc)) => out.line(
                        false,
                        1,
                        "TOOLTIP_POTENTIAL_TARGET_RAID_DESC",
                        localise(
                            loc,
                            "TOOLTIP_POTENTIAL_TARGET_RAID_DESC",
                            // 141612FC0: TEXT, literal at 14292F680.
                            &[("TEXT", Arg::Text(&desc))],
                        ),
                    ),
                    Ok(None) => {}
                    Err(reason) => out.refuse("TOOLTIP_POTENTIAL_TARGET_RAID_DESC", reason),
                }
            }
            match kind.allowed {
                Ok(false) => lines(&mut out, 1, kind.denial),
                Ok(true) if !compact => match kind.launch_condition {
                    Ok(Some(reasons)) => {
                        out.line(
                            false,
                            2,
                            "TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION",
                            plain("TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION"),
                        );
                        lines(&mut out, 1, reasons);
                    }
                    Ok(None) => {}
                    Err(reason) => out.refuse("TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION", reason),
                },
                _ => {}
            }
            if kind.range > 0 {
                out.line(
                    false,
                    2,
                    "RAID_RANGE_FACTOR_TOOLTIP",
                    localise(
                        loc,
                        "RAID_RANGE_FACTOR_TOOLTIP",
                        &[("RANGE_FACTOR", Arg::Fixed5(kind.range))],
                    ),
                );
            }
            if kind.once {
                out.line(
                    false,
                    2,
                    "tooltip_raid_only_launch_once",
                    plain("tooltip_raid_only_launch_once"),
                );
            }
        }
    }
    match quick {
        Ok(true) => out.line(
            false,
            2,
            "RAID_SHIFT_CLICK_QUICK_START",
            plain("RAID_SHIFT_CLICK_QUICK_START"),
        ),
        Ok(false) => {}
        Err(reason) => out.refuse("RAID_SHIFT_CLICK_QUICK_START", reason),
    }
    out
}
