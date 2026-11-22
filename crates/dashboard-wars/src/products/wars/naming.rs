use super::{
    WarName,
    inputs::{Goal, Held, Hour, RunWars},
    native::Pair,
};
use dashboard_base::catalogue::{self, Catalogue};
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::localisation::Argument;

pub(super) fn unknown(reason: impl Into<String>) -> WarName {
    WarName {
        text: "Not recorded".into(),
        known: false,
        reason: Some(reason.into()),
        key: None,
    }
}

pub(super) struct Facts<'a> {
    pub cat: &'a Catalogue,
    pub inputs: &'a RunWars,
    pub prefix: Option<&'a [u8]>,
    pub at: i32,
}

fn sides(hour: &Hour, p: Pair) -> (u32, u32, u32) {
    let relation = &hour.world.relations[&p];
    if relation.first_was_instigator != 0 {
        (relation.first_country, relation.second_country, 0)
    } else {
        (relation.second_country, relation.first_country, 1)
    }
}

pub(super) fn first_goal(inputs: &RunWars, hour: &Hour, p: Pair, at: i32) -> Result<Option<Goal>, String> {
    let (_, _, side) = sides(hour, p);
    let owned = hour
        .goal_owned
        .get(&p)
        .ok_or("Not recorded: representative's owned goal list")?;
    let count = if side == 0 {
        owned.first_goal_count
    } else {
        owned.second_goal_count
    };
    if count == 0 {
        return Ok(None);
    }
    let key = u128::from(owned.owner) << 96 | u128::from(owned.ordinal) << 64 | u128::from(side) << 32;
    match inputs.goals.at(key, at) {
        Some(Held::Value(goal)) => Ok(Some(*goal)),
        Some(Held::Unknown) => Err("Not recorded: a cell of the instigator's first war goal".into()),
        Some(Held::Gone) | None => Err("Not recorded: instigator's first war goal".into()),
    }
}

pub(super) fn goal_country(hour: &Hour, goal: &Goal) -> Result<Option<u32>, String> {
    if goal.resolved == 0 || goal.country_count == 0 {
        return Ok(None);
    }
    if goal.first_country == 0 {
        return Ok(Some(0));
    }
    hour.raw_to_slot
        .get(&goal.first_country)
        .copied()
        .map(Some)
        .ok_or_else(|| "Not recorded: first goal's country naming link".into())
}

pub(super) fn country(names: &CountryNames<'_>, slot: u32) -> Option<String> {
    names
        .name(slot)
        .and_then(|name| catalogue::plain_text(&name))
        .ok()
}

struct Shared {
    instigator: u32,
    defender: u32,
    extra: String,
    extra_exists: bool,
    pair_key: String,
    pair_exists: bool,
    prefix: String,
    arguments: Result<Arguments, String>,
}

struct Arguments {
    first: String,
    second: String,
    first_country: String,
    second_country: String,
    state: Option<String>,
    country: Option<(String, String)>,
}

pub(super) fn name(facts: &Facts<'_>, names: &CountryNames<'_>, hour: &Hour, p: Pair) -> WarName {
    let shared = match shared_part(facts, names, hour, p) {
        Ok(shared) => shared,
        Err(reason) => return unknown(reason),
    };
    let allied = |target: u32| -> Result<bool, String> {
        if hour.view == target {
            return Ok(true);
        }
        if hour.world.active(hour.view, target) {
            return Ok(false);
        }
        if hour
            .world
            .factions
            .get(&hour.view)
            .is_some_and(|f| *f != 0 && hour.world.factions.get(&target) == Some(f))
        {
            return Ok(true);
        }
        if hour.recorded_view != Some(hour.view) {
            return Err("Not recorded: cached subject naming predicate for the selected viewpoint (only the recording client's is captured)".into());
        }
        hour.world
            .scores
            .get(&target)
            .filter(|s| s.score_available == 1)
            .map(|s| s.view_subject != 0)
            .ok_or_else(|| "Not recorded: cached subject naming predicate".into())
    };
    let extra = if shared.extra_exists {
        let either = match allied(shared.instigator) {
            Ok(false) => allied(shared.defender),
            either => either,
        };
        match either {
            Ok(extra) => extra,
            Err(reason) => return unknown(reason),
        }
    } else {
        false
    };
    let key = if extra {
        &shared.extra
    } else if shared.pair_exists {
        &shared.pair_key
    } else {
        &shared.prefix
    };
    let arguments = match &shared.arguments {
        Ok(arguments) => arguments,
        Err(reason) => return unknown(reason.clone()),
    };
    match text(facts.cat, key, arguments) {
        Ok(text) => WarName {
            text,
            known: true,
            reason: None,
            key: Some(key.clone()),
        },
        Err(reason) => unknown(reason),
    }
}

fn text(cat: &Catalogue, key: &str, arguments: &Arguments) -> Result<String, String> {
    let mut held = vec![
        ("FIRST", Argument::Text(&arguments.first)),
        ("SECOND", Argument::Text(&arguments.second)),
        ("FIRST_COUNTRY", Argument::Text(&arguments.first_country)),
        ("SECOND_COUNTRY", Argument::Text(&arguments.second_country)),
        ("ORDER", Argument::Text("")),
    ];
    if let Some(state) = &arguments.state {
        held.push(("STATE", Argument::Text(state)));
    }
    if let Some((name, adj)) = &arguments.country {
        held.push(("COUNTRY", Argument::Text(name)));
        held.push(("COUNTRY_ADJ", Argument::Text(adj)));
    }
    catalogue::localised_with(&cat.localisation, key, &held, catalogue::View::Plain)
}

fn state_name(facts: &Facts<'_>, id: u32) -> Result<String, String> {
    let (custom, owner) = match facts.inputs.states.at(u128::from(id), facts.at) {
        Some(Held::Value(names)) => names,
        Some(Held::Unknown) => return Err(format!("state {id}: states: a name cell is not recorded")),
        Some(Held::Gone) | None => return Err(format!("state {id}: states: no row at this hour")),
    };
    dashboard_text::scripted_text::plain_state_name_of(&facts.cat.localisation, facts.cat.scripted_content()?, id, custom, owner)
}

fn shared_part(facts: &Facts<'_>, names: &CountryNames<'_>, hour: &Hour, p: Pair) -> Result<Shared, String> {
    let cat = facts.cat;
    let localisation = &cat.localisation;
    let (instigator, defender, _) = sides(hour, p);
    let goal = first_goal(facts.inputs, hour, p, facts.at)?.filter(|goal| goal.resolved != 0);
    let prefix = facts.prefix.ok_or("native war naming prefix is missing")?;
    let mut prefix = std::str::from_utf8(prefix).map_err(|_| "native war naming prefix is not UTF-8")?;
    if let Some(goal) = goal {
        let token = match facts.inputs.tokens.at(u128::from(goal.type_token), facts.at) {
            Some(Held::Value(literal)) => literal,
            _ => return Err("Not recorded: war goal type token".into()),
        };
        let token = std::str::from_utf8(token).map_err(|_| "war goal type is not UTF-8")?;
        let override_key = cat.war_names()?.prefix(token)?;
        if !override_key.is_empty() {
            prefix = override_key;
        }
    }
    let first_tag = &hour.tags[&instigator];
    let second_tag = &hour.tags[&defender];
    let pair_key = format!("{prefix}_{first_tag}_{second_tag}");
    let extra = format!("{pair_key}_{second_tag}");
    let arguments = (|| -> Result<Arguments, String> {
        let first = names.name_with(instigator, "_ADJ")?;
        let second = names.name_with(defender, "_ADJ")?;
        let first_country = names.name(instigator)?;
        let second_country = names.name(defender)?;
        let mut state = String::new();
        let mut country = None;
        if let Some(goal) = goal {
            if goal.state_count != 0 {
                state = state_name(facts, goal.first_state)?;
            }
            if let Some(slot) = goal_country(hour, &goal)? {
                country = Some((names.name(slot)?, names.name_with(slot, "_ADJ")?));
            }
        }
        Ok(Arguments {
            first,
            second,
            first_country,
            second_country,
            state: goal.map(|_| state),
            country,
        })
    })();
    Ok(Shared {
        instigator,
        defender,
        extra_exists: localisation.get(&extra).is_some(),
        extra,
        pair_exists: localisation.get(&pair_key).is_some(),
        pair_key,
        prefix: prefix.to_owned(),
        arguments,
    })
}
