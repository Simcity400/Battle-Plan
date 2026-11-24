use super::{loc, province::Facts};
use crate::content::Content;
use crate::modifier_text::{Block, Definitions};
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::localisation::{Argument, crt_case_text};
use replay_data::rows::{Recorded, tables};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::tooltip_text::ensure_newlines;

pub(crate) const OWNER_INFO_MODIFIERS: [&str; 3] = ["non_core", "non_core_controller", "resistance_effect_base"];
const NON_CORE: usize = 0;
const NON_CORE_CONTROLLER: usize = 1;
const RESISTANCE: usize = 2;

#[derive(Debug, PartialEq, Eq)]
struct Plan {
    heading: &'static str,
    modifiers: Vec<usize>,
    non_core: bool,
}

fn plan(core: bool, owner_core: bool, occupied: bool) -> Plan {
    let heading = if occupied {
        "STATE_OWNER_OCCUPIED"
    } else if core {
        "STATE_OWNER_CORE"
    } else {
        "STATE_OWNER_NON_CORE"
    };
    let mut modifiers = Vec::new();
    if occupied {
        modifiers.push(RESISTANCE);
    }
    if !core {
        modifiers.push(NON_CORE_CONTROLLER);
    }
    if !owner_core {
        modifiers.push(NON_CORE);
    }
    Plan { heading, modifiers, non_core: !core }
}

fn info_key(non_core: bool, shown: bool) -> Option<&'static str> {
    match (non_core, shown) {
        (true, true) => Some("STATE_OWNER_RESISTANCE_NON_CORE_INFO_WITH_MODIFIERS"),
        (true, false) => Some("STATE_OWNER_RESISTANCE_NON_CORE_INFO"),
        (false, true) => Some("STATE_OWNER_MODIFIER_INFO"),
        (false, false) => None,
    }
}

fn holds_core(facts: &Facts<'_>, cores: &[u32], country: u32) -> Result<bool, String> {
    for &core in cores {
        if core == country || (core != 0 && country != 0 && facts.slot(core)? == facts.slot(country)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn cores<'f>(frame: &'f DecodedFrame, state: u32) -> Result<&'f [u32], String> {
    frame
        .complete::<tables::StateCores>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.state == state)
        .map(|row| row.cores.as_slice())
        .ok_or_else(|| format!("state {state} has no StateCores row"))
}

pub(super) fn marked(facts: &Facts<'_>, names: &CountryNames<'_>, raw: u32) -> Result<String, String> {
    let flag = crate::binding::country_flag(facts.flags, raw)?;
    let name = crt_case_text(&names.name_with(facts.slot(raw)?, "_DEF")?, true, true);
    Ok(format!("{}{flag} {name}", crate::INLINE_FLAG))
}

fn tag(names: &CountryNames<'_>, slot: u32) -> Result<String, String> {
    names
        .operands(slot)
        .ok_or_else(|| format!("country {slot} has no naming operands"))?
        .as_ref()
        .map(|operands| operands.tag.clone())
        .map_err(Clone::clone)
}

pub(super) fn append(content: &Content, frame: &DecodedFrame, facts: &Facts<'_>, names: &CountryNames<'_>, country: u32, text: &mut String) -> Result<(), String> {
    ensure_newlines(text, 2);
    let state = facts.state;
    let cores = cores(frame, state.id)?;
    let resistance = frame
        .complete::<tables::StateResistance>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.state == state.id)
        .ok_or_else(|| format!("state {} has no StateResistance row", state.id))?;
    let occupied = (resistance.country as i32) > 0 && resistance.active != 0;
    let plan = plan(holds_core(facts, cores, country)?, holds_core(facts, cores, state.owner)?, occupied);
    text.push_str(&loc(content, plan.heading, &[])?);
    ensure_newlines(text, 2);
    let definitions = Definitions::read(frame, &content.modifier_names)
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let blocks = content.assets.owner_info_modifiers.as_ref().map_err(Clone::clone)?;
    let mut sum = Block::default();
    for index in plan.modifiers {
        let (block, _) = Block::build(&definitions, &blocks[index]).map_err(|e| e.to_string())?;
        super::hovers::add_block(&mut sum, block, OWNER_INFO_MODIFIERS[index])?;
    }
    let shown = sum.kept(&definitions).map_err(|e| e.to_string())?;
    if let Some(key) = info_key(plan.non_core, shown) {
        let controller = marked(facts, names, state.controller)?;
        let mut arguments = vec![("COUNTRY", Argument::Text(&controller))];
        if plan.non_core {
            let manpower = *content.assets.non_core_manpower.as_ref().map_err(Clone::clone)?;
            arguments.push(("MANPOWER", Argument::Fixed5(manpower)));
        }
        text.push_str(&loc(content, key, &arguments)?);
    }
    if shown {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        let described = sum
            .describe(&definitions, content.assets.as_ref(), &content.localisation, crate::fixed::ONE, "  ", u32::MAX)
            .map_err(|e| e.to_string())?;
        text.push_str(&described);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ControllerWeights {
    pub pairs: Vec<(u32, i32)>,
    pub total: i32,
}

pub(super) fn controller_weights(_frame: &DecodedFrame, _state: &replay_data::rows::StateRow) -> Result<ControllerWeights, String> {
    Err("the state's controller weights are not recorded".into())
}

pub(super) fn controller_shown(facts: &Facts<'_>, weights: &ControllerWeights) -> Result<bool, String> {
    let controller = facts.state.controller;
    if (controller as i32) <= 0 || weights.pairs.is_empty() || weights.total == 0 {
        return Ok(false);
    }
    let slot = facts.slot(controller)?;
    let mut weight = 0;
    for &(country, value) in &weights.pairs {
        if country == controller || (country != 0 && facts.slot(country)? == slot) {
            weight = value;
            break;
        }
    }
    Ok(weight != weights.total)
}

fn share(weight: i32, total: i32) -> i64 {
    let total = i64::from(total) * 100_000;
    let weight = i64::from(weight);
    if total == 0 || weight * 100_000 < -0x7fff_ffff_ffff_fffe {
        return 0xffff_ffff;
    }
    weight * 10_000_000_000 / total
}

fn breakdown(content: &Content, facts: &Facts<'_>, names: &CountryNames<'_>, weights: &ControllerWeights) -> Result<String, String> {
    let mut text = loc(content, "OCCUPATION_BREAKDOWN", &[])?;
    for &(country, weight) in &weights.pairs {
        let name = marked(facts, names, country)?;
        text.push_str(&loc(
            content,
            "OCCUPATION_BREAKDOWN_ENTRY",
            &[("COUNTRY", Argument::Text(&name)), ("OCCUPATION", Argument::Fixed5(share(weight, weights.total)))],
        )?);
    }
    Ok(text)
}

pub(super) fn controller_hover(content: &Content, frame: &DecodedFrame, facts: &Facts<'_>, weights: &ControllerWeights) -> Result<String, String> {
    let names = CountryNames::read(frame, &content.localisation);
    let controller = facts.state.controller;
    let slot = facts.slot(controller)?;
    let tag = tag(&names, slot)?;
    let name = names.name(slot)?;
    let mut text = loc(content, "STATE_CONTROLLER", &[("TAG", Argument::Text(&tag)), ("COUNTRY", Argument::Text(&name))])?;
    append(content, frame, facts, &names, controller, &mut text)?;
    ensure_newlines(&mut text, 2);
    text.push_str(&breakdown(content, facts, &names, weights)?);
    ensure_newlines(&mut text, 2);
    Ok(text)
}

pub(super) fn contested_owners(state: &replay_data::rows::StateRow) -> Result<&[u32], String> {
    match &state.marked_owners {
        Recorded::Value(Some(owners)) => Ok(owners),
        Recorded::Value(None) => Err("the state's contested owner list was refused when read".into()),
        Recorded::NotRecorded => Err("the state's contested owner list is not recorded".into()),
    }
}

pub(super) fn contested_hover(content: &Content, names: &CountryNames<'_>, facts: &Facts<'_>, country: u32, state: &str) -> Result<String, String> {
    let name = names.name(facts.slot(country)?)?;
    loc(content, "STATE_CONTESTED_OWNER", &[("COUNTRY", Argument::Text(&name)), ("STATE", Argument::Text(state))])
}

pub(super) fn owner_hover(content: &Content, frame: &DecodedFrame, facts: &Facts<'_>, names: &CountryNames<'_>) -> Result<String, String> {
    let owner = facts.state.owner;
    let slot = facts.slot(owner)?;
    let tag = tag(names, slot)?;
    let name = names.name(slot)?;
    let mut text = loc(content, "STATE_OWNER", &[("TAG", Argument::Text(&tag)), ("COUNTRY", Argument::Text(&name))])?;
    text.push('\n');
    if slot != facts.slot(facts.state.controller)? {
        ensure_newlines(&mut text, 2);
        let key = if holds_core(facts, cores(frame, facts.state.id)?, owner)? { "STATE_OWNER_CORE" } else { "STATE_OWNER_NON_CORE" };
        text.push_str(&loc(content, key, &[])?);
    } else {
        append(content, frame, facts, names, owner, &mut text)?;
    }
    ensure_newlines(&mut text, 2);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contested_owners_are_the_states_recorded_marked_owners() {
        use replay_data::rows::StateRow;
        let state = |marked_owners| StateRow { marked_owners, ..Default::default() };
        assert_eq!(contested_owners(&state(Recorded::Value(Some(vec![4, 2])))), Ok(&[4, 2][..]));
        assert_eq!(contested_owners(&state(Recorded::Value(Some(Vec::new())))), Ok(&[][..]));
        assert_eq!(contested_owners(&state(Recorded::Value(None))), Err("the state's contested owner list was refused when read".to_owned()));
        assert_eq!(contested_owners(&state(Recorded::NotRecorded)), Err("the state's contested owner list is not recorded".to_owned()));
    }

    #[test]
    fn the_heading_and_modifiers_follow_core_ownership_and_occupation() {
        assert_eq!(plan(true, true, false), Plan { heading: "STATE_OWNER_CORE", modifiers: vec![], non_core: false });
        assert_eq!(
            plan(false, false, false),
            Plan { heading: "STATE_OWNER_NON_CORE", modifiers: vec![NON_CORE_CONTROLLER, NON_CORE], non_core: true }
        );
        assert_eq!(
            plan(false, true, true),
            Plan { heading: "STATE_OWNER_OCCUPIED", modifiers: vec![RESISTANCE, NON_CORE_CONTROLLER], non_core: true }
        );
        assert_eq!(plan(true, false, false).modifiers, vec![NON_CORE]);
    }

    fn with_facts<T>(owner: u32, controller: u32, test: impl FnOnce(&Facts<'_>) -> T) -> T {
        let province = replay_data::rows::ProvinceRow::default();
        let state = replay_data::rows::StateRow { owner, controller, ..Default::default() };
        let links = [(1, 10), (2, 20), (3, 20)].into();
        let flags = Default::default();
        test(&Facts { province: &province, state: &state, links: &links, flags: &flags })
    }

    #[test]
    fn the_controller_flag_shows_while_the_controller_holds_less_than_the_whole_weight() {
        let weights = |pairs: Vec<(u32, i32)>, total| ControllerWeights { pairs, total };
        with_facts(1, 2, |facts| {
            assert_eq!(controller_shown(facts, &weights(vec![(2, 7), (1, 3)], 10)), Ok(true));
            assert_eq!(controller_shown(facts, &weights(vec![(1, 0), (2, 10)], 10)), Ok(false));
            assert_eq!(controller_shown(facts, &weights(vec![(3, 10), (2, 4)], 10)), Ok(false));
            assert_eq!(controller_shown(facts, &weights(vec![], 10)), Ok(false));
            assert_eq!(controller_shown(facts, &weights(vec![(1, 4)], 0)), Ok(false));
            assert_eq!(controller_shown(facts, &weights(vec![(1, 10)], 10)), Ok(true));
        });
        with_facts(1, 0, |facts| assert_eq!(controller_shown(facts, &weights(vec![(1, 4)], 10)), Ok(false)));
    }
}
