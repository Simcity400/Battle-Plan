use super::{paths_named, withhold_named};
use crate::binding::Bindings;
use crate::content::Content;
use content_reader_gui::gui::native;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_runtime::resource_origins::{self, Definition};

const GRID: &str = "state_resources_entries";
const ENTRY_HOVER: &str = "state view resource hover: its amount breakdown lists the sources of the state's resource modifiers, which are not recorded";

pub(super) fn bind(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, state: u32) {
    let amounts = content
        .air_context
        .as_ref()
        .ok_or_else(|| "the state content is not loaded".to_owned())
        .and_then(|context| context.province_hover.resources().map_err(str::to_owned))
        .and_then(|resources| resource_origins::state_amounts(frame, resources, state));
    match amounts {
        Ok(amounts) => {
            let mut entries: Vec<crate::Values> = amounts.iter().filter_map(|(definition, amount)| entry(out, definition, *amount)).collect();
            if !entries.is_empty() {
                for values in &mut entries {
                    crate::tooltips::refuse_hovers(values, ["resource", "amount"]);
                }
                out.refuse(ENTRY_HOVER.into());
            }
            for path in paths_named(root, GRID) {
                let widget = out.values.entry(path).or_default();
                widget.entry_template = Some(super::entry::RESOURCE.into());
                widget.entries = entries.clone();
            }
        }
        Err(why) => withhold_named(out, root, &[GRID], &format!("state view resources: {why}")),
    }
}

pub(super) fn none_text(frame: &DecodedFrame, state: u32) -> Result<bool, String> {
    let listed = frame.complete::<tables::StateBaseResources>().map_err(|e| e.to_string())?.iter().any(|row| row.state == state);
    let pooled = frame.complete::<tables::StateResources>().map_err(|e| e.to_string())?.iter().any(|row| row.state == state && row.pool > 0);
    let gate = frame
        .complete::<tables::StateResourceInputs>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.state == state)
        .map(|row| row.gate)
        .ok_or_else(|| format!("state {state} has no StateResourceInputs row"))?;
    Ok(none(listed, pooled, gate))
}

fn none(listed: bool, pooled: bool, gate: u8) -> bool {
    !listed && !pooled && gate == 0
}

fn entry(out: &mut Bindings, definition: &Definition, amount: i64) -> Option<crate::Values> {
    if amount == 0 {
        return None;
    }
    let mut values = crate::Values::new();
    match definition.icon_frame {
        Some(frame) => values.entry("resource".into()).or_default().frame = Some(frame - 1),
        None => out.withhold_in(&mut values, &["resource"], format!("state view resource {}: its declaration has no icon frame", definition.name)),
    }
    crate::binding::text(&mut values, "amount", round(amount).to_string());
    Some(values)
}

fn round(value: i64) -> i64 {
    let (whole, rest) = (value / 100_000, value % 100_000);
    if rest > 49_999 {
        whole + 1
    } else if rest < -49_999 {
        whole - 1
    } else {
        whole
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(icon_frame: Option<u32>) -> Definition {
        Definition { index: 1, token: 7, name: "steel".into(), icon_frame }
    }

    #[test]
    fn only_a_state_without_listed_pooled_or_gated_resources_has_none() {
        assert!(none(false, false, 0));
        assert!(!none(true, false, 0));
        assert!(!none(false, true, 0));
        assert!(!none(false, false, 1));
    }

    #[test]
    fn zero_amounts_are_left_out_and_others_round_halves_away_from_zero() {
        let mut out = Bindings::default();
        assert!(entry(&mut out, &definition(Some(3)), 0).is_none());
        let shown = entry(&mut out, &definition(Some(3)), 1_250_000).unwrap();
        assert_eq!(shown["resource"].frame, Some(2));
        assert_eq!(shown["amount"].text.as_deref(), Some("13"));
        assert_eq!(round(1_249_999), 12);
        assert_eq!(round(-150_000), -2);
        assert_eq!(round(40_000), 0);
    }

    #[test]
    fn a_resource_without_an_icon_frame_withholds_only_its_icon() {
        let mut out = Bindings::default();
        let shown = entry(&mut out, &definition(None), 200_000).unwrap();
        assert!(shown["resource"].hidden);
        assert_eq!(shown["amount"].text.as_deref(), Some("2"));
        assert_eq!(out.refused().len(), 1);
    }
}
