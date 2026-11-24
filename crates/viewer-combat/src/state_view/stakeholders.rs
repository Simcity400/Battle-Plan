use super::{loc, paths_named, province::Facts, withhold_named};
use crate::binding::{self, Bindings};
use crate::content::Content;
use crate::tooltips::{hover_text, set_hover};
use content_reader_gui::gui::native;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;

const FLAG: &str = "stakeholder_country_flag";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Stake,
    Owner,
    Contested,
}

fn entry(flag: String, kind: Kind) -> crate::Values {
    let mut values = crate::Values::new();
    values.entry(FLAG.into()).or_default().flag = Some(flag);
    for (name, shown) in [
        ("flag_border_thin", kind != Kind::Owner),
        ("flag_border_thick", kind == Kind::Owner),
        ("flag_exclamation_mark", kind == Kind::Contested),
    ] {
        if shown {
            values.entry(name.into()).or_default().hidden = false;
        } else {
            binding::hide(&mut values, name);
        }
    }
    values
}

fn entry_hover(values: &mut crate::Values, text: Result<String, String>, refused: &mut Vec<String>) {
    match text {
        Ok(text) => set_hover(values, FLAG, hover_text(&text), None),
        Err(why) => {
            set_hover(values, FLAG, None, None);
            refused.push(why);
        }
    }
}

fn hover(loc: impl Fn(&str, &[(&str, Argument<'_>)]) -> Result<String, String>, core: bool, country: &str, state: &str) -> Result<(String, String), String> {
    let arguments = [("COUNTRY", Argument::Text(country)), ("STATE", Argument::Text(state))];
    if core {
        Ok((loc("STATE_CORE", &arguments)?, loc("STATE_CORE_DESC", &[])? + "\n"))
    } else {
        Ok((loc("STATE_CLAIM", &arguments)?, String::new()))
    }
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    root: &native::Element,
    out: &mut Bindings,
    facts: &Facts<'_>,
    state_name: &Result<String, String>,
) {
    let state = facts.state;
    let names = content_reader_country_names::country_names::CountryNames::read(frame, &content.localisation);
    let mut refused = Vec::new();
    let claims = (|| -> Result<Vec<crate::Values>, String> {
        let rows = frame.complete::<tables::StateCores>().map_err(|e| e.to_string())?;
        let row = rows
            .iter()
            .find(|row| row.state == state.id)
            .ok_or_else(|| format!("state {} has no StateCores row", state.id))?;
        let Recorded::Value(claims) = &row.claims else {
            return Err("the state's claims are not recorded".into());
        };
        let owner = facts.slot(state.owner)?;
        let mut entries = Vec::new();
        let cores = row.cores.len();
        for (at, &country) in row.cores.iter().chain(claims).enumerate() {
            let slot = facts.slot(country)?;
            if slot == owner {
                continue;
            }
            let mut values = entry(binding::country_flag(facts.flags, country)?, Kind::Stake);
            let text = state_name
                .clone()
                .and_then(|state| hover(|key, arguments| loc(content, key, arguments), at < cores, &names.name(slot)?, &state))
                .map_err(|why| format!("state view stakeholder hover: {why}"));
            match text {
                Ok((main, delayed)) => set_hover(&mut values, FLAG, hover_text(&main), hover_text(&delayed)),
                Err(why) => {
                    set_hover(&mut values, FLAG, None, None);
                    refused.push(why);
                }
            }
            entries.push(values);
        }
        Ok(entries)
    })();
    match claims {
        Ok(entries) => {
            for path in paths_named(root, "state_claims") {
                let widget = out.values.entry(path).or_default();
                widget.entry_template = Some(super::entry::STAKEHOLDER.into());
                widget.entries = entries.clone();
            }
        }
        Err(why) => withhold_named(out, root, &["state_claims"], &format!("state view cores and claims: {why}")),
    }
    let owners = (|| -> Result<Vec<crate::Values>, String> {
        let contested = super::owner::contested_owners(state)?;
        let mut entries = Vec::new();
        for &country in contested {
            let mut values = entry(binding::country_flag(facts.flags, country)?, Kind::Contested);
            let text = state_name
                .clone()
                .and_then(|state| super::owner::contested_hover(content, &names, facts, country, &state))
                .map_err(|why| format!("state view contested owner hover: {why}"));
            entry_hover(&mut values, text, &mut refused);
            entries.push(values);
        }
        let mut values = entry(binding::country_flag(facts.flags, state.owner)?, Kind::Owner);
        let text = super::owner::owner_hover(content, frame, facts, &names).map_err(|why| format!("state view owner hover: {why}"));
        entry_hover(&mut values, text, &mut refused);
        entries.push(values);
        Ok(entries)
    })();
    let size = (|| -> Result<[f32; 2], String> {
        let name = if facts.slot(state.owner)? == facts.slot(state.controller)? {
            "state_owners_size_when_owner_is_controller"
        } else {
            "state_owners_size_when_owner_is_not_controller"
        };
        let element = content.assets.native_windows.gui_type(name.as_bytes()).ok_or_else(|| format!("the content declares no {name}"))?;
        let position = element.position_type().ok_or_else(|| format!("{name} is not a position"))?.position;
        Ok(position.map(|value| value as f32))
    })();
    for why in refused {
        out.refuse(why);
    }
    match owners.and_then(|entries| Ok((entries, size?))) {
        Ok((entries, size)) => {
            for path in paths_named(root, "state_owners") {
                let widget = out.values.entry(path).or_default();
                widget.entry_template = Some(super::entry::STAKEHOLDER.into());
                widget.entries = entries.clone();
                widget.size = Some(size);
            }
        }
        Err(why) => withhold_named(out, root, &["state_owners"], &format!("state view owners: {why}")),
    }
    let controller = super::owner::controller_weights(frame, state)
        .and_then(|weights| Ok((super::owner::controller_shown(facts, &weights)?, weights)));
    match controller {
        Ok((true, weights)) => match binding::country_flag(facts.flags, state.controller) {
            Ok(flag) => {
                let text = super::owner::controller_hover(content, frame, facts, &weights)
                    .map_err(|why| format!("state view controller flag hover: {why}"));
                for path in paths_named(root, "controller_flag") {
                    let widget = out.values.entry(path.clone()).or_default();
                    widget.hidden = false;
                    widget.flag = Some(flag.clone());
                    match &text {
                        Ok(text) => set_hover(&mut out.values, path, hover_text(text), None),
                        Err(why) => {
                            set_hover(&mut out.values, path, None, None);
                            out.refuse(why.clone());
                        }
                    }
                }
            }
            Err(why) => withhold_named(out, root, &["controller_flag", "controller_flag_border"], &format!("state view controller flag: {why}")),
        },
        Ok((false, _)) => {
            for name in ["controller_flag", "controller_flag_border"] {
                for path in paths_named(root, name) {
                    binding::hide(&mut out.values, &path);
                }
            }
        }
        Err(why) => withhold_named(out, root, &["controller_flag", "controller_flag_border"], &format!("state view controller flag: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_core_hover_adds_its_description_and_a_claim_hover_has_none() {
        let loc = |key: &str, arguments: &[(&str, Argument<'_>)]| {
            let named: Vec<String> = arguments
                .iter()
                .map(|(name, value)| match value {
                    Argument::Text(text) => format!("{name}={text}"),
                    _ => unreachable!(),
                })
                .collect();
            Ok(format!("{key}[{}]", named.join(",")))
        };
        assert_eq!(hover(loc, true, "Germany", "Saxony"), Ok(("STATE_CORE[COUNTRY=Germany,STATE=Saxony]".into(), "STATE_CORE_DESC[]\n".into())));
        assert_eq!(hover(loc, false, "Germany", "Saxony"), Ok(("STATE_CLAIM[COUNTRY=Germany,STATE=Saxony]".into(), String::new())));
    }
}
