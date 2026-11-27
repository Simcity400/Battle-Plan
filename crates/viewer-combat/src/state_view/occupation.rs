use super::{paths_named, province::Facts, withhold_named};
use crate::binding::{self, Bindings};
use crate::content::Content;
use crate::tooltips::{hover_text, set_hover};
use content_reader_localisation::localisation::Argument;
use dashboard_text::occupation_law::{self, GroupLaw, LawInputs};
use content_reader_politics::occupation_laws::Law;
use viewer_foundation::tooltip_text::ensure_newlines;
use content_reader_gui::gui::native;
use replay_data::rows::{GarrisonTemplateSelection, Recorded, StateResistanceRow, tables};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::templates::{counter_custom_icon, custom_icon_index};

const CONTAINER: &str = "resistance_container";
const LAW_AND_TEMPLATE: [&str; 3] = ["select_law_icon", "template_garrison", "template_need_text"];

#[expect(clippy::too_many_arguments, reason = "the window's read inputs, each from another owner")]
pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    root: &native::Element,
    out: &mut Bindings,
    facts: &Facts<'_>,
    viewer: u32,
    nato: bool,
    state_name: &Result<String, String>,
) {
    let state = facts.state.id;
    let resistance = frame
        .complete::<tables::StateResistance>()
        .map_err(|e| e.to_string())
        .and_then(|rows| rows.iter().find(|row| row.state == state).ok_or_else(|| format!("state {state} has no StateResistance row")));
    let resistance = match resistance {
        Ok(row) if row.active == 0 => {
            for path in paths_named(root, CONTAINER) {
                binding::hide(&mut out.values, &path);
            }
            return;
        }
        Ok(row) => row,
        Err(why) => {
            withhold_named(out, root, &[CONTAINER], &format!("state view resistance: {why}"));
            return;
        }
    };
    bars(content, root, out, resistance);
    let controller = facts.slot(facts.state.controller);
    match controller {
        Ok(controller) if controller == viewer => {}
        Ok(_) => {
            for path in paths_named(root, "law_and_template_container") {
                binding::hide(&mut out.values, &path);
            }
            return;
        }
        Err(why) => {
            withhold_named(out, root, &["law_and_template_container"], &format!("state view occupation law and garrison: {why}"));
            return;
        }
    }
    let occupation = content.occupation.as_ref().as_ref().map_err(Clone::clone);
    let law = occupation.clone().and_then(|occupation| law(content, frame, facts, resistance, &occupation.laws));
    let hover = match &law {
        Ok(picked) => match &picked.paired {
            Ok(true) => facts
                .slot(facts.state.controller)
                .and_then(|controller| Ok((controller, facts.slot(resistance.country)?)))
                .and_then(|(controller, occupied)| law_hover(content, frame, picked.law, controller, occupied))
                .map(Some),
            Ok(false) => Ok(None),
            Err(why) => Err(why.clone()),
        },
        Err(why) => Err(why.clone()),
    };
    match law.and_then(|picked| icon_frame(picked.law).map(|frame| (picked.law, frame))) {
        Ok((law, frame_index)) => {
            content.assets.acquire_sprite(&law.sprite);
            for path in paths_named(root, LAW_AND_TEMPLATE[0]) {
                let widget = out.values.entry(path).or_default();
                widget.hidden = false;
                widget.sprite = Some(law.sprite.clone());
                widget.retain_sprite = false;
                widget.frame = Some(frame_index);
            }
        }
        Err(why) => withhold_named(out, root, &[LAW_AND_TEMPLATE[0]], &format!("state view occupation law: {why}")),
    }
    for path in paths_named(root, "select_law_button") {
        match &hover {
            Ok(Some(text)) => set_hover(&mut out.values, path, hover_text(text), None),
            Ok(None) => {}
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(format!("state view occupation law hover: {why}"));
            }
        }
    }
    match garrison_icon(content, frame, facts, resistance, nato) {
        Ok((sprite, frame_index)) => {
            for path in paths_named(root, LAW_AND_TEMPLATE[1]) {
                let widget = out.values.entry(path).or_default();
                widget.hidden = false;
                widget.sprite = Some(sprite.clone());
                widget.retain_sprite = false;
                widget.frame = Some(frame_index);
            }
        }
        Err(why) => withhold_named(out, root, &[LAW_AND_TEMPLATE[1]], &format!("state view garrison template: {why}")),
    }
    let hover = garrison_template(frame, facts, resistance)
        .and_then(|template| viewer_counter_products::map::province_tooltips::template_display_name(&content.localisation, frame, template))
        .and_then(|template| {
            let state = state_name.clone()?;
            super::loc(
                content,
                "OCCUPATION_CHANGE_GARRISON_STATE_TEMPLATE_TOOLTIP",
                &[("SELECTED_TEMPLATE", Argument::Text(&template)), ("OCCUPIED_STATE", Argument::Text(&state))],
            )
        })
        .map_err(|why| format!("state view garrison template hover: {why}"));
    for path in paths_named(root, "template_garrison_button") {
        match &hover {
            Ok(text) => set_hover(&mut out.values, path, hover_text(text), None),
            Err(why) => {
                set_hover(&mut out.values, path, None, None);
                out.refuse(why.clone());
            }
        }
    }
    let need = required(frame, facts).map(need_text);
    for path in paths_named(root, LAW_AND_TEMPLATE[2]) {
        out.bind(&path, binding::TEXT, need.clone().map_err(|why| format!("state view garrison need: {why}")));
    }
}

const BARS: [(&str, &str); 2] = [
    ("resistance_status_pos", super::entry::RESISTANCE),
    ("compliance_status_pos", super::entry::COMPLIANCE),
];

fn bar_percent(value: i64) -> i32 {
    crate::fixed::round(value).clamp(0, 100)
}

fn bar_values(percent: i32) -> crate::Values {
    let mut values = crate::Values::new();
    values.entry("progress".into()).or_default().frame = Some(percent.max(1) as u32 - 1);
    binding::text(&mut values, "progress_text", content_reader_parser::fixed5::fixed5_percent(i64::from(percent) * 100_000, 0, false));
    values
}

fn bars(content: &Content, root: &native::Element, out: &mut Bindings, resistance: &StateResistanceRow) {
    for ((position, template), value) in BARS.into_iter().zip([resistance.level, resistance.compliance]) {
        let mut values = bar_values(bar_percent(value));
        if let Some(entry) = content.assets.native_windows.gui_type(template.as_bytes()) {
            let markers: Vec<&str> = ["modifier_container", "resistance_target"]
                .into_iter()
                .filter(|name| !paths_named(entry, name).is_empty())
                .collect();
            if !markers.is_empty() {
                out.withhold_in(&mut values, &markers, format!("state view {template} markers: the occupation modifier threshold markers and the suppression target are not ported to the state view"));
            }
        }
        for path in paths_named(root, position) {
            let widget = out.values.entry(path).or_default();
            widget.entry_template = Some(template.into());
            widget.entries = vec![values.clone()];
        }
    }
}

struct Picked<'l> {
    law: &'l Law,
    paired: Result<bool, String>,
}

fn law<'l>(
    content: &Content,
    frame: &DecodedFrame,
    facts: &Facts<'_>,
    resistance: &StateResistanceRow,
    laws: &'l content_reader_politics::occupation_laws::Laws,
) -> Result<Picked<'l>, String> {
    let state = facts.state.id;
    let controller = facts.slot(facts.state.controller)?;
    let occupied = facts.slot(resistance.country)?;
    let overrides = frame.complete::<tables::StateOccupationLaws>();
    let overrides_complete = overrides.is_ok();
    let mut state_law = None;
    for row in overrides.unwrap_or_default() {
        if row.state == state && facts.slot(row.controller) == Ok(controller) && facts.slot(row.occupied) == Ok(occupied) {
            state_law = Some(row.name.as_slice());
        }
    }
    let settings = frame.complete::<tables::OccupationSettings>().map_err(|e| e.to_string())?;
    let mut mine = settings.iter().filter(|row| facts.slot(row.country) == Ok(controller));
    let settings = match (mine.next(), mine.next()) {
        (Some(row), None) => Some(row),
        (None, _) => None,
        (Some(_), Some(_)) => return Err("occupation settings hold two rows for the occupier".into()),
    };
    let mut scope = None;
    let mut paired = Ok(false);
    let group = match settings {
        None => GroupLaw::Unknown("The occupier's occupation settings are missing".into()),
        Some(row) => match &row.group_laws {
            Recorded::NotRecorded => GroupLaw::Unknown("the law the occupier set for the occupied country is not recorded".into()),
            Recorded::Value(None) => GroupLaw::Unknown("The occupied-country law inventory capture failed".into()),
            Recorded::Value(Some(groups)) => match groups.iter().find(|group| facts.slot(group.occupied) == Ok(occupied)) {
                Some(group) => {
                    scope = group.scope_country;
                    paired = Ok(true);
                    GroupLaw::Law(group.law.as_deref())
                }
                None => GroupLaw::Missing,
            },
        },
    };
    if let GroupLaw::Unknown(why) = &group {
        paired = Err(why.clone());
    }
    let fallback = settings.ok_or_else(|| "The native database fallback law is missing".to_owned()).and_then(|row| match &row.fallback_law {
        Recorded::Value(Some(bytes)) => Ok(bytes.as_slice()),
        Recorded::Value(None) => Err("The native database fallback-law capture failed".into()),
        Recorded::NotRecorded => Err("The native database fallback law is not recorded".into()),
    });
    let inputs = LawInputs {
        state: state_law,
        key_resolved: true,
        overrides_complete,
        group,
        default: settings.map(|row| (row.default_law_present != 0, row.law.as_slice())),
        fallback,
    };
    let token = occupation_law::law_for(inputs, |token| {
        let definition = laws.find(token)?.ok_or_else(|| format!("occupation law {token} has no content declaration"))?;
        if definition.visible.is_empty() && definition.available.is_empty() {
            return Ok(true);
        }
        let scoped = scope
            .ok_or_else(|| "the occupied-country scope is not captured".to_owned())
            .and_then(|raw| facts.slot(raw))?;
        if definition.visible.is_empty() && controller == scoped {
            return Ok(true);
        }
        let world = dashboard_text::triggers::World::new(frame, content.trigger_content.facts()).map_err(|why| why.to_string())?;
        occupation_law::admitted(&world, definition, controller, scoped)
    })?;
    let law = laws.find(&token)?.ok_or_else(|| format!("occupation law {token} has no content declaration"))?;
    Ok(Picked { law, paired })
}

fn icon_frame(law: &Law) -> Result<u32, String> {
    u32::try_from(law.frame)
        .ok()
        .and_then(|frame| frame.checked_sub(1))
        .ok_or_else(|| format!("occupation law {}'s icon frame is not positive", law.token))
}

fn law_hover(content: &Content, frame: &DecodedFrame, law: &Law, controller: u32, occupied: u32) -> Result<String, String> {
    use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
    let mut text = format!("\u{11}H{}\u{11}!", super::loc(content, &law.token, &[])?);
    ensure_newlines(&mut text, 2);
    if !law.tooltip.is_empty() {
        text.push_str(&super::loc(content, &law.tooltip, &[])?);
    }
    ensure_newlines(&mut text, 2);
    if !law.available.is_empty() {
        let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
        let scripted = dashboard_text::scripted_text::ScriptedText::new(frame, &content.localisation, scripted, controller).viewing(controller);
        let world = dashboard_text::triggers::World::new(frame, content.trigger_content.facts()).map_err(|why| why.to_string())?;
        let mut occupier = EventScope::of(EventObject::Country(controller));
        occupier.from = Link::Scope(1);
        let mut target = EventScope::of(EventObject::Country(occupied));
        target.root = Link::Scope(0);
        let graph = std::rc::Rc::new(std::cell::RefCell::new(EventScopes::new(vec![occupier, target])));
        let evaluator = dashboard_text::triggers::Evaluator::new(&world, None).with_event_scopes(graph);
        let scopes = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
        let mut refused = std::collections::BTreeMap::new();
        let described = dashboard_text::triggers::description::describe(
            &law.available,
            &evaluator,
            &scripted,
            &content.localisation,
            scopes,
            &[],
            &|_: &[u8]| -> Result<Vec<u8>, String> { Err("an occupation law trigger names no tactic".into()) },
            &mut refused,
        )?;
        if let Some(why) = refused.keys().next() {
            return Err(format!("its availability trigger: {why}"));
        }
        text.push_str(&viewer_foundation::tooltip_text::displayable(content.assets.as_ref(), described)?);
        ensure_newlines(&mut text, 2);
    }
    let definitions = crate::modifier_text::Definitions::read(frame, &content.modifier_names)
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let modifiers = |items: &[content_reader_parser::parser::Item]| -> Result<String, String> {
        let (block, _) = crate::modifier_text::Block::build(&definitions, &content_reader_script::modifiers::values(items)).map_err(|e| e.to_string())?;
        block
            .describe_at(&definitions, content.assets.as_ref(), &content.localisation, crate::fixed::ONE, "  ", u32::MAX, 3)
            .map_err(|e| e.to_string())
    };
    text.push_str(&super::loc(content, "OCCUPATION_POLICY_STATE_MODIFIER", &[])?);
    text.push('\n');
    text.push_str(&modifiers(&law.state_modifier)?);
    ensure_newlines(&mut text, 2);
    if let Some(suppressed) = &law.suppressed_state_modifier {
        text.push_str(&super::loc(content, "OCCUPATION_POLICY_SUPRESSED_STATE_MODIFIER", &[])?);
        text.push('\n');
        text.push_str(&modifiers(suppressed)?);
        ensure_newlines(&mut text, 2);
    }
    Ok(text)
}

fn selected_template(selections: &[GarrisonTemplateSelection], occupied: impl Fn(u32) -> bool, state: u32) -> Option<u64> {
    let find = |wanted_state: Option<u32>, group: bool| {
        selections
            .iter()
            .find(|selection| selection.state == wanted_state && selection.occupied.map_or(!group, |raw| group && occupied(raw)))
            .and_then(|selection| selection.template)
    };
    find(Some(state), true).or_else(|| find(None, true)).or_else(|| find(None, false))
}

fn garrison_template(frame: &DecodedFrame, facts: &Facts<'_>, resistance: &StateResistanceRow) -> Result<u64, String> {
    let controller = facts.slot(facts.state.controller)?;
    let occupied = facts.slot(resistance.country)?;
    let rows = frame.complete::<tables::CountryTemplateState>().map_err(|e| e.to_string())?;
    let row = rows
        .iter()
        .find(|row| facts.slot(row.country) == Ok(controller))
        .ok_or("the occupier has no CountryTemplateState row")?;
    let selections = match &row.garrisons {
        Recorded::Value(Some(selections)) => selections,
        Recorded::Value(None) => return Err("the occupier's garrison template selections were refused when recorded".into()),
        Recorded::NotRecorded => return Err("the occupier's garrison template selections are not recorded".into()),
    };
    selected_template(selections, |raw| facts.slot(raw) == Ok(occupied), facts.state.id)
        .ok_or_else(|| "no selection is set, and the game's last fallback template is not recorded".into())
}

fn garrison_icon(content: &Content, frame: &DecodedFrame, facts: &Facts<'_>, resistance: &StateResistanceRow, nato: bool) -> Result<(String, u32), String> {
    let template = garrison_template(frame, facts, resistance)?;
    let icons = frame.complete::<tables::DivisionTemplateIcons>().map_err(|e| e.to_string())?;
    let icon = icons
        .iter()
        .find(|row| row.template == template)
        .ok_or_else(|| format!("template {template} has no DivisionTemplateIcons row"))?;
    if let Some(index) = custom_icon_index(counter_custom_icon(icon.custom_icon, icon.custom_icon_nato, nato)) {
        let sprite = format!("GFX_div_templ_{index}_large");
        if content.assets.has_sprite(&sprite) {
            return Ok((sprite, 0));
        }
    }
    let sprite = format!("GFX_unit_{}_icon_medium", String::from_utf8_lossy(&icon.priority_unit_name));
    if !content.assets.has_sprite(&sprite) {
        return Err(format!("the content declares no {sprite}, so the game keeps the previous icon"));
    }
    Ok((sprite, u32::from(nato)))
}

fn required(frame: &DecodedFrame, facts: &Facts<'_>) -> Result<i64, String> {
    let controller = facts.slot(facts.state.controller)?;
    frame
        .complete::<tables::StateGarrisons>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.state == facts.state.id && facts.slot(row.controller) == Ok(controller))
        .map(|row| row.garrison_required)
        .ok_or_else(|| format!("state {} has no StateGarrisons row for its controller", facts.state.id))
}

fn need_text(amount: i64) -> String {
    match amount {
        ..0 => "\u{11}R ! \u{11}!".into(),
        0 => " - ".into(),
        _ => content_reader_parser::fixed5::fixed5_to_string(amount, 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(occupied: Option<u32>, state: Option<u32>, template: Option<u64>) -> GarrisonTemplateSelection {
        GarrisonTemplateSelection { occupied, state, template }
    }

    #[test]
    fn the_state_selection_wins_then_the_occupied_country_then_the_default() {
        let all = [selection(None, None, Some(1)), selection(Some(9), None, Some(2)), selection(Some(9), Some(40), Some(3)), selection(Some(8), Some(41), Some(4))];
        let ninth = |raw| raw == 9;
        assert_eq!(selected_template(&all, ninth, 40), Some(3));
        assert_eq!(selected_template(&all, ninth, 41), Some(2));
        assert_eq!(selected_template(&all, |raw| raw == 7, 41), Some(1));
        let unset = [selection(None, None, Some(1)), selection(Some(9), None, None), selection(Some(9), Some(40), None)];
        assert_eq!(selected_template(&unset, ninth, 40), Some(1));
        assert_eq!(selected_template(&[selection(None, None, None)], ninth, 40), None);
    }

    #[test]
    fn bars_round_their_value_into_a_hundred_frames() {
        assert_eq!(bar_percent(4_249_999), 42);
        assert_eq!(bar_percent(4_250_000), 43);
        assert_eq!(bar_percent(-300_000), 0);
        assert_eq!(bar_percent(12_000_000), 100);
        let values = bar_values(0);
        assert_eq!(values["progress"].frame, Some(0));
        assert_eq!(values["progress_text"].text.as_deref(), Some("0%"));
        assert_eq!(bar_values(42)["progress"].frame, Some(41));
        assert_eq!(bar_values(42)["progress_text"].text.as_deref(), Some("42%"));
    }

}
