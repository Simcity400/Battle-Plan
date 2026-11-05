//! War names use content-defined types and overrides;
//! the capture supplies the native default prefix and ordered first goal.
use super::*;
use dashboard_text::scripted_text::ScriptedText;
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::Argument;
use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};

pub(super) fn unknown(reason: impl Into<String>) -> WarName {
    WarName {
        text: "Not recorded".into(),
        known: false,
        reason: Some(reason.into()),
        key: None,
    }
}

/// The name `hour`'s viewpoint gives the war of relation `p` at `frame`, with `cat`.
#[cfg(test)]
pub(super) fn name(frame: &DecodedFrame, hour: &Hour, p: Pair, cat: Option<&Catalogue>) -> WarName {
    Namer::new(frame, cat).name(hour, p).0
}

/// What a relation's name reads alike for every viewpoint: its localisation keys and its
/// arguments, each part's failure kept for the place it is reported in.
struct Shared {
    instigator: u32,
    defender: u32,
    /// The key with the defender's tag appended, read only where it exists.
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
    /// The first goal's state name: none without a resolved goal.
    state: Option<String>,
    country: Option<(String, String)>,
}

/// One hour's naming, for every viewpoint naming at it, with one catalogue. Rows ascend by key,
/// so lookups search them; the first war named builds the shared contexts, and each relation's
/// viewpoint-independent parts are read once.
pub(super) struct Namer<'a> {
    frame: &'a DecodedFrame,
    cat: Option<&'a Catalogue>,
    goals: Result<&'a [rows::WarRelationGoalRow], String>,
    tokens: Result<&'a [rows::LexicalTokenRow], String>,
    prefix: Result<&'a str, String>,
    names: OnceCell<CountryNames<'a>>,
    /// Plain state names read the frame and content, never the context's root.
    states: OnceCell<ScriptedText<'a>>,
    shared: RefCell<foldhash::HashMap<Pair, Rc<Result<Shared, String>>>>,
    texts: RefCell<HashMap<(Pair, String), Result<String, String>>>,
}

impl<'a> Namer<'a> {
    pub fn new(frame: &'a DecodedFrame, cat: Option<&'a Catalogue>) -> Self {
        Self {
            frame,
            cat,
            goals: frame
                .complete::<tables::WarRelationGoals>()
                .map_err(|e| e.to_string()),
            tokens: frame
                .complete::<tables::LexicalTokens>()
                .map_err(|e| e.to_string()),
            prefix: frame
                .complete::<tables::WarNameRules>()
                .map_err(|e| e.to_string())
                .and_then(|rows| match rows {
                    [row] if row.index == 0 => std::str::from_utf8(&row.prefix)
                        .map_err(|_| "native war naming prefix is not UTF-8".into()),
                    _ => Err("native war naming prefix is missing".into()),
                }),
            names: OnceCell::new(),
            states: OnceCell::new(),
            shared: RefCell::default(),
            texts: RefCell::default(),
        }
    }

    /// The name `hour`'s viewpoint gives the war of relation `p`, and whether naming it read the
    /// war overview's scores.
    pub fn name(&self, hour: &Hour, p: Pair) -> (WarName, bool) {
        let held = self.shared.borrow().get(&p).cloned();
        let shared = held.unwrap_or_else(|| {
            let shared = Rc::new(self.shared_part(hour, p));
            self.shared.borrow_mut().insert(p, Rc::clone(&shared));
            shared
        });
        let shared = match &*shared {
            Ok(shared) => shared,
            Err(reason) => return (unknown(reason.clone()), false),
        };
        let mut scored = false;
        let allied = |target: u32, scored: &mut bool| -> Result<bool, String> {
            // Check self, the war veto,
            // nonnull faction equality, then either cached subject endpoint.
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
            *scored = true;
            hour.world
                .scores
                .get(&target)
                .filter(|s| s.score_available == 1)
                .map(|s| s.view_subject != 0)
                .ok_or_else(|| "Not recorded: cached subject naming predicate".into())
        };
        // Both alliance branches append the defender tag. Without that key the
        // predicate cannot change the name, so it is read only when the key exists.
        let extra = if shared.extra_exists {
            let either = match allied(shared.instigator, &mut scored) {
                Ok(false) => allied(shared.defender, &mut scored),
                either => either,
            };
            match either {
                Ok(extra) => extra,
                Err(reason) => return (unknown(reason), scored),
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
            Err(reason) => return (unknown(reason.clone()), scored),
        };
        let text = self
            .texts
            .borrow_mut()
            .entry((p, key.clone()))
            .or_insert_with(|| self.text(key, arguments))
            .clone();
        let name = match text {
            Ok(text) => WarName {
                text,
                known: true,
                reason: None,
                key: Some(key.clone()),
            },
            Err(reason) => unknown(reason),
        };
        (name, scored)
    }

    fn text(&self, key: &str, arguments: &Arguments) -> Result<String, String> {
        let cat = self
            .cat
            .ok_or("Not recorded: the recorded content stack is not resolved")?;
        let mut held = vec![
            ("FIRST", Argument::Text(&arguments.first)),
            ("SECOND", Argument::Text(&arguments.second)),
            ("FIRST_COUNTRY", Argument::Text(&arguments.first_country)),
            ("SECOND_COUNTRY", Argument::Text(&arguments.second_country)),
            ("ORDER", Argument::Text("")),
        ];
        // The recorded naming rule supplies no ordinal suffix.
        if let Some(state) = &arguments.state {
            held.push(("STATE", Argument::Text(state)));
        }
        if let Some((name, adj)) = &arguments.country {
            held.push(("COUNTRY", Argument::Text(name)));
            held.push(("COUNTRY_ADJ", Argument::Text(adj)));
        }
        catalogue::localised_with(&cat.localisation, key, &held, catalogue::View::Plain)
    }

    /// What naming relation `p` reads alike for every viewpoint of `hour`, or the first failure
    /// before the viewpoint's alliance is read.
    fn shared_part(&self, hour: &Hour, p: Pair) -> Result<Shared, String> {
        let cat = self
            .cat
            .ok_or("Not recorded: the recorded content stack is not resolved")?;
        let localisation = &cat.localisation;
        let names = self
            .names
            .get_or_init(|| CountryNames::read(self.frame, localisation));
        let relation = &hour.world.relations[&p];
        let (instigator, defender, side) = if relation.first_was_instigator != 0 {
            (relation.first_country, relation.second_country, 0)
        } else {
            (relation.second_country, relation.first_country, 1)
        };
        let owned = hour
            .goal_owned
            .get(&p)
            .ok_or("Not recorded: representative's owned goal list")?;
        let count = if side == 0 {
            owned.first_goal_count
        } else {
            owned.second_goal_count
        };
        let goal = if count == 0 {
            None
        } else {
            let goals = self.goals.as_ref().map_err(Clone::clone)?;
            let first = goals
                .binary_search_by_key(&(owned.owner, owned.ordinal, side, 0), |r| {
                    (r.owner, r.relation_ordinal, r.side, r.ordinal)
                })
                .map_err(|_| "Not recorded: instigator's first war goal")?;
            Some(&goals[first])
        };
        let goal = goal.filter(|r| r.resolved != 0);
        let mut prefix = self.prefix.clone()?;
        if let Some(goal) = goal {
            let tokens = self.tokens.as_ref().map_err(Clone::clone)?;
            let token = tokens
                .binary_search_by_key(&goal.type_token, |r| r.token)
                .map_err(|_| "Not recorded: war goal type token")?;
            let token = std::str::from_utf8(&tokens[token].literal)
                .map_err(|_| "war goal type is not UTF-8")?;
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
                    let scripted = cat.scripted_content()?;
                    state = self
                        .states
                        .get_or_init(|| {
                            ScriptedText::new(self.frame, localisation, scripted, hour.view)
                        })
                        .plain_state_name(goal.first_state)?;
                }
                if goal.country_count != 0 {
                    // Reference zero selects the reserved country without a link.
                    let slot = if goal.first_country == 0 {
                        0
                    } else {
                        *hour
                            .raw_to_slot
                            .get(&goal.first_country)
                            .ok_or("Not recorded: first goal's country naming link")?
                    };
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
}
