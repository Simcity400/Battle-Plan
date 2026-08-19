
use replay_core::{AuthenticatedUiPass, HudPaintCommand, Leaf, UiPassOperation, WorldFrame};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedUiOwner {
    pub country: Option<String>,
    pub runtime_text_pov_country_or_global: String,
}

pub fn recorded_ui_owner(
    frame: &WorldFrame,
    pass: &AuthenticatedUiPass,
) -> Result<RecordedUiOwner, String> {
    let frame_hour = u64::try_from(frame.hour_index())
        .map_err(|_| "recorded UI owner cannot join a negative frame hour")?;
    if pass.update_index != frame_hour {
        return Err(format!(
            "recorded UI pass update {} differs from WorldFrame hour {frame_hour}",
            pass.update_index
        ));
    }

    let text_scope = recorded_text_scope(pass)?;
    let mut country_candidates = BTreeSet::<String>::new();

    if text_scope != "global" {
        insert_country(&mut country_candidates, &text_scope, "A25 UI-pass scope")?;
    }

    if let Leaf::Measured(companion) = &frame.diplomacy().intelligence_controller_companion {
        if companion.frame_hour != frame_hour
            || companion.selected_present_receipt_sha256 != pass.selected_present_receipt_sha256
        {
            return Err("A30 UI owner companion does not name the current selected Present".into());
        }
        insert_country(
            &mut country_candidates,
            &companion.owner,
            "A30 controller companion",
        )?;
    }

    let visibility_povs = frame.visibility_povs().collect::<Vec<_>>();
    if visibility_povs.len() == 1 {
        insert_country(
            &mut country_candidates,
            visibility_povs[0],
            "d94 recorded POV",
        )?;
    }

    let country = match country_candidates.len() {
        0 => None,
        1 => country_candidates.into_iter().next(),
        _ => {
            return Err(format!(
                "authenticated selected-Present carriers disagree on UI owner: {:?}",
                country_candidates
            ))
        }
    };
    Ok(RecordedUiOwner {
        country,
        runtime_text_pov_country_or_global: text_scope,
    })
}

fn recorded_text_scope(pass: &AuthenticatedUiPass) -> Result<String, String> {
    let mut scopes = BTreeSet::new();
    for operation in &pass.ui_pass_operations {
        let UiPassOperation::HudCommand { command } = operation else {
            continue;
        };
        let replay_core::HudCommandExecution::InterpretedPaint {
            paint:
                HudPaintCommand::Text {
                    runtime_text_pov_country_or_global,
                    ..
                },
        } = &command.execution
        else {
            continue;
        };
        if runtime_text_pov_country_or_global == "global" {
            scopes.insert("global".to_owned());
        } else {
            if !valid_country(runtime_text_pov_country_or_global) {
                return Err(format!(
                    "selected-Present text carries invalid recorded POV scope {:?}",
                    runtime_text_pov_country_or_global
                ));
            }
            scopes.insert(runtime_text_pov_country_or_global.clone());
        }
    }
    match scopes.len() {
        0 => Ok("global".into()),
        1 => Ok(scopes.into_iter().next().expect("one text scope")),
        _ => Err(format!(
            "selected-Present text operations disagree on recorded A25 scope: {:?}",
            scopes
        )),
    }
}

fn insert_country(
    candidates: &mut BTreeSet<String>,
    value: &str,
    source: &str,
) -> Result<(), String> {
    if !valid_country(value) {
        return Err(format!(
            "{source} carries invalid country identity {value:?}"
        ));
    }
    candidates.insert(value.to_owned());
    Ok(())
}

fn valid_country(value: &str) -> bool {
    value.len() == 3
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}
