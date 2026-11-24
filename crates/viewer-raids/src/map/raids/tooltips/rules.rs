use super::Tooltip;
use engine_native_air::craid_instance::{OUTCOME_NONE, known_outcome_key, known_phase_key};
use content_reader_localisation::localisation::Argument as Arg;
use content_reader_localisation::localisation::Localisation;
use viewer_foundation::tooltip_text::localise;

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
    pub gate: (Known<bool>, Lines),
    pub assigned_unit: Known<Option<Text>>,
    pub owner: Text,
    pub success: Known<i64>,
    pub counterplay: Known<Vec<Text>>,
}

pub fn raid(loc: &Localisation, input: Raid, incoming: bool) -> Tooltip {
    let mut out = Tooltip::default();
    let plain = |key| localise(loc, key, &[]);
    if input.ended {
        let outcome = match input.outcome {
            OUTCOME_NONE => {
                out.refuse(TOOLTIP_RAID_RESULT, "raid outcome was not recorded".to_owned());
                Ok("raid outcome was not recorded".to_owned())
            }
            code => match known_outcome_key(code) {
                Some(key) => plain(key),
                None => {
                    out.refuse(TOOLTIP_RAID_RESULT, format!("unsupported raid outcome ({code})"));
                    Ok(format!("Unsupported outcome ({code})"))
                }
            },
        };
        out.line(
            false,
            0,
            TOOLTIP_RAID_RESULT,
            (|| {
                localise(
                    loc,
                    TOOLTIP_RAID_RESULT,
                    &[
                        ("RAID_DESC", Arg::Text(&input.description?)),
                        ("OUTCOME_TYPE", Arg::Text(&outcome?)),
                    ],
                )
            })(),
        );
        out.line(
            true,
            0,
            TOOLTIP_RAID_VIEW_DETAILS_PROMPT,
            plain(TOOLTIP_RAID_VIEW_DETAILS_PROMPT),
        );
        out.line(
            true,
            1,
            TOOLTIP_RAID_CLOSE_REPORT_PROMPT,
            plain(TOOLTIP_RAID_CLOSE_REPORT_PROMPT),
        );
        return out;
    }
    if incoming {
        out.line(
            false,
            0,
            TOOLTIP_RAID_WARNING_HEADER,
            plain(TOOLTIP_RAID_WARNING_HEADER),
        );
        out.line(
            false,
            1,
            TOOLTIP_RAID_WARNING_DESCRIPTION,
            (|| {
                localise(
                    loc,
                    TOOLTIP_RAID_WARNING_DESCRIPTION,
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
            TOOLTIP_RAID_WARNING_SUCCESS_CHANCE_HEADER,
            input.success.and_then(|chance| {
                localise(
                    loc,
                    TOOLTIP_RAID_WARNING_SUCCESS_CHANCE_HEADER,
                    &[("VALUE", Arg::Fixed5(chance))],
                )
            }),
        );
        match input.counterplay {
            Ok(lines) if !lines.is_empty() => {
                out.line(
                    false,
                    2,
                    TOOLTIP_RAID_WARNING_COUNTERPLAY_HEADER,
                    plain(TOOLTIP_RAID_WARNING_COUNTERPLAY_HEADER),
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
    out.line(
        true,
        0,
        TOOLTIP_RAID_VIEW_DETAILS_PROMPT,
        plain(TOOLTIP_RAID_VIEW_DETAILS_PROMPT),
    );
    out.line(false, 1, RAID_INST_ITEM_DESC, input.description);
    let phase = match known_phase_key(input.phase) {
        Some(key) => plain(key),
        None => {
            out.refuse(
                RAID_INST_PROGRESS_DESC,
                format!("unsupported raid phase ({})", input.phase),
            );
            Ok(format!("Unsupported phase ({})", input.phase))
        }
    };
    out.line(
        false,
        2,
        RAID_INST_PROGRESS_DESC,
        input.progress.and_then(|progress| {
            let phase = phase?;
            match progress {
                Some(value) => localise(
                    loc,
                    RAID_INST_PROGRESS_DESC,
                    &[
                        ("PHASE", Arg::Text(&phase)),
                        ("PROGRESS", Arg::Fixed5(value)),
                    ],
                ),
                None => Ok(phase),
            }
        }),
    );
    if input.phase == 1 || input.phase == 3 {
        match input.gate {
            (allowed, reasons) if input.phase == 1 || allowed == Ok(false) => {
                if input.phase == 3 {
                    out.line(
                        false,
                        2,
                        TOOLTIP_LAUNCH_BUTTON_DISABLED,
                        plain(TOOLTIP_LAUNCH_BUTTON_DISABLED),
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
            TOOLTIP_LAUNCH_BUTTON_RIGHT_CLICK,
            name.and_then(|name| {
                localise(
                    loc,
                    TOOLTIP_LAUNCH_BUTTON_RIGHT_CLICK,
                    &[("NAME", Arg::Text(&name))],
                )
            }),
        ),
        Ok(None) => {}
        Err(reason) => out.refuse(TOOLTIP_LAUNCH_BUTTON_RIGHT_CLICK, reason),
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

pub fn offers(loc: &Localisation, targets: Vec<Target>, quick: Known<bool>) -> Tooltip {
    let compact = targets.len() != 1 || targets[0].types.len() != 1;
    let mut out = Tooltip::default();
    let plain = |key| localise(loc, key, &[]);
    for target in targets {
        let header = TOOLTIP_POTENTIAL_TARGET_ENTRY;
        out.line(
            false,
            2,
            header,
            (|| {
                let key = if target.allowed? {
                    header
                } else {
                    TOOLTIP_POTENTIAL_TARGET_ENTRY_NOT_ALLOWED
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
                TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC,
                localise(
                    loc,
                    TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC,
                    &[
                        ("CURRENT", Arg::Fixed5(current)),
                        ("REQUIRED", Arg::Fixed5(required)),
                    ],
                ),
            ),
            Ok(Facility::Project(name)) => out.line(
                false,
                1,
                TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC,
                name.and_then(|name| {
                    localise(
                        loc,
                        TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC,
                        &[("PROJECT_NAME", Arg::Text(&name))],
                    )
                }),
            ),
            Ok(Facility::Idle) => out.line(
                false,
                1,
                TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC,
                plain(TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC),
            ),
            Err(reason) => out.refuse("TOOLTIP_TARGET_FACILITY_PROJECT", reason),
        }
        if !unknown {
            match target.detection {
                Ok(Some(current)) => out.line(
                    false,
                    1,
                    TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL,
                    localise(
                        loc,
                        TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL,
                        &[("CURRENT", Arg::Fixed5(current))],
                    ),
                ),
                Ok(None) => {}
                Err(reason) => out.refuse(TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL, reason),
            }
        }
        if target.types.len() > 1 {
            out.line(
                false,
                2,
                TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS,
                plain(TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS),
            );
        }
        for kind in target.types {
            out.line(
                false,
                2,
                "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER",
                (|| {
                    let key = if kind.allowed.clone()? {
                        TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_ALLOWED
                    } else {
                        TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_NOT_ALLOWED
                    };
                    localise(loc, key, &[("RAID", Arg::Text(&kind.name?))])
                })(),
            );
            match kind.captive {
                Ok(Some(name)) => out.line(
                    false,
                    1,
                    RAID_TYPE_RESCUE_CAPTURED_GENERAL_CAPTIVE_LINE,
                    localise(
                        loc,
                        RAID_TYPE_RESCUE_CAPTURED_GENERAL_CAPTIVE_LINE,
                        &[("NAME", Arg::Text(&name))],
                    ),
                ),
                Ok(None) => {}
                Err(reason) => out.refuse(RAID_TYPE_RESCUE_CAPTURED_GENERAL_CAPTIVE_LINE, reason),
            }
            if !compact {
                match kind.description {
                    Ok(Some(desc)) => out.line(
                        false,
                        1,
                        TOOLTIP_POTENTIAL_TARGET_RAID_DESC,
                        localise(
                            loc,
                            TOOLTIP_POTENTIAL_TARGET_RAID_DESC,
                            &[("TEXT", Arg::Text(&desc))],
                        ),
                    ),
                    Ok(None) => {}
                    Err(reason) => out.refuse(TOOLTIP_POTENTIAL_TARGET_RAID_DESC, reason),
                }
            }
            match kind.allowed {
                Ok(false) => lines(&mut out, 1, kind.denial),
                Ok(true) if !compact => match kind.launch_condition {
                    Ok(Some(reasons)) => {
                        out.line(
                            false,
                            2,
                            TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION,
                            plain(TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION),
                        );
                        lines(&mut out, 1, reasons);
                    }
                    Ok(None) => {}
                    Err(reason) => out.refuse(TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION, reason),
                },
                _ => {}
            }
            if kind.range > 0 {
                out.line(
                    false,
                    2,
                    RAID_RANGE_FACTOR_TOOLTIP,
                    localise(
                        loc,
                        RAID_RANGE_FACTOR_TOOLTIP,
                        &[("RANGE_FACTOR", Arg::Fixed5(kind.range))],
                    ),
                );
            }
            if kind.once {
                out.line(
                    false,
                    2,
                    TOOLTIP_RAID_ONLY_LAUNCH_ONCE,
                    plain(TOOLTIP_RAID_ONLY_LAUNCH_ONCE),
                );
            }
        }
    }
    match quick {
        Ok(true) => out.line(
            false,
            2,
            RAID_SHIFT_CLICK_QUICK_START,
            plain(RAID_SHIFT_CLICK_QUICK_START),
        ),
        Ok(false) => {}
        Err(reason) => out.refuse(RAID_SHIFT_CLICK_QUICK_START, reason),
    }
    out
}

const TOOLTIP_RAID_RESULT: &str = "tooltip_raid_result";
const TOOLTIP_RAID_VIEW_DETAILS_PROMPT: &str = "tooltip_raid_view_details_prompt";
const TOOLTIP_RAID_CLOSE_REPORT_PROMPT: &str = "tooltip_raid_close_report_prompt";
const TOOLTIP_RAID_WARNING_HEADER: &str = "tooltip_raid_warning_header";
const TOOLTIP_RAID_WARNING_DESCRIPTION: &str = "tooltip_raid_warning_description";
const TOOLTIP_RAID_WARNING_SUCCESS_CHANCE_HEADER: &str =
    "tooltip_raid_warning_success_chance_header";
const TOOLTIP_RAID_WARNING_COUNTERPLAY_HEADER: &str = "tooltip_raid_warning_counterplay_header";
const RAID_INST_ITEM_DESC: &str = "raid_inst_item_desc";
const RAID_INST_PROGRESS_DESC: &str = "raid_inst_progress_desc";
const TOOLTIP_LAUNCH_BUTTON_DISABLED: &str = "tooltip_launch_button_disabled";
const TOOLTIP_LAUNCH_BUTTON_RIGHT_CLICK: &str = "tooltip_launch_button_right_click";
const TOOLTIP_POTENTIAL_TARGET_ENTRY: &str = "TOOLTIP_POTENTIAL_TARGET_ENTRY";
const TOOLTIP_POTENTIAL_TARGET_ENTRY_NOT_ALLOWED: &str =
    "TOOLTIP_POTENTIAL_TARGET_ENTRY_NOT_ALLOWED";
const TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC: &str =
    "TOOLTIP_TARGET_FACILITY_PROJECT_UNKNOWN_DESC";
const TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC: &str =
    "TOOLTIP_TARGET_FACILITY_ONGOING_PROJECT_DESC";
const TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC: &str =
    "TOOLTIP_TARGET_FACILITY_NO_ONGOING_PROJECT_DESC";
const TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL: &str = "TOOLTIP_POTENTIAL_TARGET_ENTRY_INTEL";
const TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS: &str = "TOOLTIP_POTENTIAL_TARGET_MULTIPLE_RAIDS";
const TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_ALLOWED: &str =
    "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_ALLOWED";
const TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_NOT_ALLOWED: &str =
    "TOOLTIP_POTENTIAL_TARGET_RAID_HEADER_NOT_ALLOWED";
const RAID_TYPE_RESCUE_CAPTURED_GENERAL_CAPTIVE_LINE: &str =
    "raid_type_rescue_captured_general_captive_line";
const TOOLTIP_POTENTIAL_TARGET_RAID_DESC: &str = "TOOLTIP_POTENTIAL_TARGET_RAID_DESC";
const TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION: &str = "TOOLTIP_POTENTIAL_TARGET_LAUNCH_CONDITION";
const RAID_RANGE_FACTOR_TOOLTIP: &str = "RAID_RANGE_FACTOR_TOOLTIP";
const TOOLTIP_RAID_ONLY_LAUNCH_ONCE: &str = "tooltip_raid_only_launch_once";
const RAID_SHIFT_CLICK_QUICK_START: &str = "RAID_SHIFT_CLICK_QUICK_START";
