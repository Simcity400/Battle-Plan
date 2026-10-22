//! An exact-hour diplomacy inventory, also used beside the war overview.
use dashboard_base::{
    Source,
    catalogue::{self, View},
    products::{LoadContext, Product, check_reply_budget, validate_tag},
};
use replay::container::SeekResult;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct Wargoals;
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WargoalsParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalEntry {
    pub handle: String,
    pub name: String,
    pub actor: String,
    pub recipient: String,
    pub available: bool,
    pub expires: Option<i32>,
    pub states: Vec<u32>,
    pub liberate: Vec<String>,
    pub puppets: Vec<String>,
    pub terms: Option<WargoalTerms>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalTerm {
    pub name: String,
    pub limit: i32,
    pub cost_factor: String,
    pub threat_factor: String,
    pub conditioned: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalTerms {
    pub allowed: Option<String>,
    pub available: Option<String>,
    pub annex: bool,
    pub base_generation_cost: i32,
    pub per_state_generation_cost: i32,
    pub expiry_days: i32,
    pub threat: String,
    pub peace: Vec<WargoalTerm>,
}
impl From<&installed_content_wargoals::wargoals::Terms> for WargoalTerms {
    fn from(value: &installed_content_wargoals::wargoals::Terms) -> Self {
        Self {
            allowed: None,
            available: None,
            annex: value.annex,
            base_generation_cost: value.generate_base_cost,
            per_state_generation_cost: value.generate_per_state_cost,
            expiry_days: value.expiry_days,
            threat: value.threat.to_string(),
            peace: ["Take states", "Liberate", "Puppet", "Change government"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| WargoalTerm {
                    name: name.into(),
                    limit: value.limits[i],
                    cost_factor: value.cost_factors[i].to_string(),
                    threat_factor: value.threat_factors[i].to_string(),
                    conditioned: !value.conditions[i].is_empty(),
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalJustification {
    pub ordinal: u32,
    pub name: String,
    pub actor: String,
    pub recipient: String,
    pub states: Vec<u32>,
    pub total: String,
    pub points: String,
    pub daily: String,
    pub pay_upfront: bool,
    pub start: Option<i32>,
    pub completion: i32,
    pub threat: String,
    pub paused: Option<bool>,
    pub terms: Option<WargoalTerms>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalStateName {
    pub id: u32,
    pub name: String,
    pub known: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalEligibility {
    pub actor: String,
    pub recipient: String,
    pub justify: Option<bool>,
    pub declare: Option<bool>,
    pub justify_reason: String,
    pub declare_reason: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WargoalsReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub eligibility: Vec<WargoalEligibility>,
    pub goals: Vec<WargoalEntry>,
    pub justifications: Vec<WargoalJustification>,
    pub warnings: Vec<String>,
    pub state_names: Vec<WargoalStateName>,
}

pub const TABLES: &[TableId] = &[
    TableId::WarOwnedRelations,
    TableId::FactionMembers,
    TableId::States,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::CountryWargoals,
    TableId::WargoalTargets,
    TableId::WargoalJustifications,
];
impl Product for Wargoals {
    const KEY: &'static str = "wargoals";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = WargoalsParams;
    type Reply = WargoalsReply;
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&p.tag)?;
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid war-goal branch")?;
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            WargoalsParams::decl(),
            WargoalEntry::decl(),
            WargoalTerms::decl(),
            WargoalTerm::decl(),
            WargoalJustification::decl(),
            WargoalsReply::decl(),
            WargoalEligibility::decl(),
            WargoalStateName::decl(),
        ]
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::other("War-goal request cancelled"));
        }
        let catalogue = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options))
            .map_err(|why| why.to_string());
        let held = catalogue.as_ref().ok().map(|cat| {
            cat.content
                .layers
                .derived::<std::sync::OnceLock<Result<installed_content_wargoals::wargoals::Database, String>>>()
        });
        let definitions = match (catalogue.as_ref(), held.as_ref()) {
            (Ok(cat), Some(held)) => held
                .get_or_init(|| {
                    installed_content_wargoals::wargoals::read(&cat.content.layers)
                        .map_err(|why| why.to_string())
                })
                .as_ref()
                .map_err(Clone::clone),
            (Err(why), _) => Err(why.clone()),
            _ => Err("War-goal content is unavailable".into()),
        };
        let branch = p
            .branch_id
            .parse()
            .map_err(|_| io::Error::other("Invalid war-goal branch"))?;
        // Through the shared frame cache, which keeps a gap's reason.
        let read = |tables: &[TableId]| -> io::Result<_> {
            match dashboard_base::prepared::query::frames::seek(
                source, branch, p.hour, tables, cancelled,
            )? {
                SeekResult::Frame(frame) => Ok(frame),
                SeekResult::Gap { reason, .. } => Err(io::Error::other(format!(
                    "War goals not recorded: {reason}"
                ))),
            }
        };
        let mut tables = TABLES.to_vec();
        tables.extend(installed_content_country_names::country_names::TABLES);
        tables.sort_by_key(|t| t.index());
        tables.dedup();
        let mut frame = read(&tables)?;
        if describes_gates(&frame, &p.tag, definitions.as_ref().ok().copied()) {
            tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
            tables.sort_by_key(|t| t.index());
            tables.dedup();
            frame = read(&tables)?;
        }
        let slot = frame
            .complete::<tables::Countries>()?
            .iter()
            .find(|c| c.tag == p.tag.as_bytes())
            .map(|c| c.canonical_slot)
            .ok_or_else(|| invalid("War-goal country is absent"))?;
        let text = catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
            cat.scripted_content().map(|scripted| {
                dashboard_text::scripted_text::ScriptedText::new(
                    &frame,
                    &cat.localisation,
                    scripted,
                    slot,
                )
            })
        });
        let links: BTreeMap<_, _> = frame
            .complete::<tables::CountryLinks>()?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let mut term_warnings = Vec::new();
        let mut reply = inventory(&frame, p, |literal, actor, recipient| {
            let slots = || -> Result<(u32, u32), String> {
                Ok((
                    *links.get(&actor).ok_or("War-goal actor is unresolved")?,
                    *links
                        .get(&recipient)
                        .ok_or("War-goal target is unresolved")?,
                ))
            };
            let term = definitions
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|d| d.terms(literal));
            let terms = match term {
                Ok(term) => {
                    let count = term.conditions.iter().filter(|c| !c.is_empty()).count();
                    if count != 0 {
                        term_warnings.push(format!("War-goal term conditions refused ({count}): an inventory entry does not contain the peace action's evaluation context or a stored condition result."));
                    }
                    let mut terms = WargoalTerms::from(term);
                    for (field, items, output) in [
                        ("allowed", term.allowed.as_slice(), &mut terms.allowed),
                        ("available", term.available.as_slice(), &mut terms.available),
                    ] {
                        if items.is_empty() {
                            *output = Some(String::new());
                            continue;
                        }
                        let mut refusals = BTreeMap::new();
                        let result = (|| {
                            let (actor, recipient) = slots()?;
                            let text = text.as_ref().map_err(Clone::clone)?;
                            describe_gate(
                                items,
                                text,
                                &catalogue.as_ref().map_err(Clone::clone)?.localisation,
                                gate_scopes(actor, recipient, field == "available"),
                                &mut refusals,
                            )
                        })();
                        match result {
                            Ok(description) if refusals.is_empty() || !description.is_empty() => {
                                *output = Some(description)
                            }
                            Ok(_) => (),
                            Err(why) => *refusals.entry(why).or_default() += 1,
                        }
                        for (why, count) in refusals {
                            term_warnings.push(format!(
                                "War-goal {field} description refused ({count}): {}: {why}",
                                String::from_utf8_lossy(literal)
                            ));
                        }
                    }
                    Some(terms)
                }
                Err(why) => {
                    term_warnings.push(format!("War-goal terms refused (1): {why}"));
                    None
                }
            };
            let definition_name = definitions
                .as_ref()
                .map_err(|why| invalid(why.clone()))?
                .find(literal)
                .map_err(io::Error::other)?
                .ok_or_else(|| {
                    io::Error::other("War-goal definition is absent from the recorded content")
                })?;
            let label = (|| {
                let (actor, recipient) = slots()?;
                let cat = catalogue.as_ref().map_err(Clone::clone)?;
                localised_goal_name(&frame, &cat.localisation, definition_name, actor, recipient)
            })();
            let name = match label {
                Ok(name) => name,
                Err(why) => {
                    term_warnings.push(format!("War-goal name refused (1): {why}"));
                    "Not recorded".into()
                }
            };
            Ok((name, terms))
        })?;
        reply.warnings.extend(term_warnings);
        let ids: BTreeSet<_> = reply
            .goals
            .iter()
            .flat_map(|g| g.states.iter())
            .chain(reply.justifications.iter().flat_map(|j| j.states.iter()))
            .copied()
            .collect();
        if !ids.is_empty() {
            for id in ids {
                let result = text
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|text| text.plain_state_name(id));
                let (name, known) = match result {
                    Ok(name) => (name, true),
                    Err(why) => {
                        reply
                            .warnings
                            .push(format!("War-goal state name refused (1): {why}"));
                        ("Not recorded".into(), false)
                    }
                };
                reply.state_names.push(WargoalStateName { id, name, known });
            }
        }
        if let Ok(definitions) = definitions {
            reply.warnings.extend(definitions.notes.iter().cloned());
        }
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// Whether the selected country holds a goal or justification whose terms have
/// an allowed or available gate: describing one reads the trigger world.
fn describes_gates(
    frame: &DecodedFrame,
    tag: &str,
    definitions: Option<&installed_content_wargoals::wargoals::Database>,
) -> bool {
    let Some(definitions) = definitions else {
        return false;
    };
    let (Ok(countries), Ok(links), Ok(tokens), Ok(goals), Ok(justifications)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
        frame.complete::<tables::LexicalTokens>(),
        frame.complete::<tables::CountryWargoals>(),
        frame.complete::<tables::WargoalJustifications>(),
    ) else {
        // The inventory refuses before it describes a gate.
        return false;
    };
    let Some(selected) = countries.iter().find(|c| c.tag == tag.as_bytes()) else {
        return false;
    };
    let slots: BTreeMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
    let same = |raw: u32| slots.get(&raw) == Some(&selected.canonical_slot);
    let literals: BTreeMap<_, _> = tokens
        .iter()
        .map(|r| (r.token, r.literal.as_slice()))
        .collect();
    goals
        .iter()
        .filter(|r| same(r.country))
        .map(|r| r.goal_type)
        .chain(
            justifications
                .iter()
                .filter(|r| same(r.country))
                .map(|r| r.goal_type),
        )
        .any(|token| {
            literals.get(&token).is_some_and(|literal| {
                definitions
                    .terms(literal)
                    .is_ok_and(|term| !term.allowed.is_empty() || !term.available.is_empty())
            })
        })
}

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}
fn localised_goal_name(
    frame: &DecodedFrame,
    localisation: &installed_content_localisation::localisation::Localisation,
    definition: &[u8],
    actor: u32,
    recipient: u32,
) -> Result<String, String> {
    let names = installed_content_country_names::country_names::CountryNames::read_selected(
        frame,
        localisation,
        [actor, recipient],
    );
    let actor = names.name(actor)?;
    let target = names.name(recipient)?;
    catalogue::localised_with(
        localisation,
        std::str::from_utf8(definition).map_err(|_| "War-goal definition name is not UTF-8")?,
        &[
            (
                "ACTOR",
                installed_content_localisation::localisation::Argument::Text(&actor),
            ),
            (
                "TARGET",
                installed_content_localisation::localisation::Argument::Text(&target),
            ),
        ],
        View::Plain,
    )
}

fn gate_scopes(actor: u32, recipient: u32, available: bool) -> dashboard_text::triggers::Scopes {
    use dashboard_text::triggers::{Scope, Scopes};
    // Allowed uses the actor's fresh country scope. Available uses a fresh
    // target scope with ROOT linked to that actor scope; FROM/PREV stay self.
    let this = Scope::Country(if available { recipient } else { actor });
    Scopes {
        root: Scope::Country(actor),
        from: Some(this),
        prev: Some(this),
        ..Scopes::of(this)
    }
}

fn describe_gate<'a>(
    items: &[installed_content_core::parser::Item],
    text: &dashboard_text::scripted_text::ScriptedText<'a>,
    localisation: &installed_content_localisation::localisation::Localisation,
    scopes: dashboard_text::triggers::Scopes,
    refusals: &mut BTreeMap<String, usize>,
) -> Result<String, String> {
    if items.is_empty() {
        return Ok(String::new());
    }
    let evaluator = dashboard_text::triggers::Evaluator::new(text.trigger_world()?, None);
    dashboard_text::triggers::description::describe(
        items,
        &evaluator,
        text,
        localisation,
        scopes,
        &[],
        &|name| {
            Err(format!(
                "War-goal tactic description unavailable: {}",
                String::from_utf8_lossy(name)
            ))
        },
        refusals,
    )
    .and_then(|body| catalogue::plain_text(&body))
}

/// Only inventory pairs demand an answer. A recorded native blocker proves
/// false; the absence of that blocker cannot prove all action gates true.
fn eligibility(
    frame: &DecodedFrame,
    pairs: &BTreeSet<(String, String)>,
    warnings: &mut Vec<String>,
) -> Vec<WargoalEligibility> {
    let links = frame.complete::<tables::CountryLinks>().ok();
    let countries = frame.complete::<tables::Countries>().ok();
    let slot = |tag: &str| {
        countries
            .as_ref()?
            .iter()
            .find(|r| r.tag == tag.as_bytes())
            .map(|r| r.canonical_slot)
    };
    let canonical = |raw| {
        links
            .as_ref()?
            .iter()
            .find(|r| r.id == raw)
            .map(|r| r.canonical_slot)
    };
    let relations = frame.complete::<tables::WarOwnedRelations>().ok();
    let members = frame.complete::<tables::FactionMembers>().ok();
    pairs
        .iter()
        .map(|(actor, recipient)| {
            let known = slot(actor).zip(slot(recipient));
            let war = known.is_some_and(|(actor, recipient)| {
                relations.as_ref().is_some_and(|rows| {
                    rows.iter().any(|r| {
                        r.retired == 0
                            && canonical(r.owner) == Some(actor)
                            && ((canonical(r.first_country) == Some(actor)
                                && canonical(r.second_country) == Some(recipient))
                                || (canonical(r.second_country) == Some(actor)
                                    && canonical(r.first_country) == Some(recipient)))
                    })
                })
            });
            let faction = |slot| -> Option<u64> {
                let mut ids = members
                    .as_ref()?
                    .iter()
                    .filter(|r| canonical(r.country) == Some(slot))
                    .map(|r| r.faction);
                let first = ids.next()?;
                (first != 0 && ids.all(|id| id == first)).then_some(first)
            };
            let same_faction =
                known.is_some_and(|(a, b)| faction(a).zip(faction(b)).is_some_and(|(a, b)| a == b));
            let justify_reason = if war {
                "Active war between these countries"
            } else if same_faction {
                "Countries belong to the same faction"
            } else {
                "Remaining native justification gates are unresolved"
            };
            let declare_reason = if war {
                "Active war between these countries"
            } else if same_faction {
                "Countries belong to the same faction"
            } else {
                "Remaining native declaration gates are unresolved"
            };
            let justify = (war || same_faction).then_some(false);
            let declare = (war || same_faction).then_some(false);
            for (action, result, why) in [
                ("Justify war", justify, justify_reason),
                ("Declare war", declare, declare_reason),
            ] {
                if result.is_none() {
                    warnings.push(format!(
                        "{action} eligibility refused (1): {actor} -> {recipient}: {why}."
                    ));
                }
            }
            WargoalEligibility {
                actor: actor.clone(),
                recipient: recipient.clone(),
                justify,
                declare,
                justify_reason: justify_reason.into(),
                declare_reason: declare_reason.into(),
            }
        })
        .collect()
}

fn inventory(
    frame: &DecodedFrame,
    p: &WargoalsParams,
    mut name: impl FnMut(&[u8], u32, u32) -> io::Result<(String, Option<WargoalTerms>)>,
) -> io::Result<WargoalsReply> {
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let selected = countries
        .iter()
        .find(|c| c.tag == p.tag.as_bytes())
        .ok_or_else(|| invalid("War-goal country is absent at this hour"))?;
    let slots: BTreeMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
    let tags: BTreeMap<_, _> = countries
        .iter()
        .map(|r| (r.canonical_slot, r.tag.as_slice()))
        .collect();
    let same = |raw: u32| slots.get(&raw) == Some(&selected.canonical_slot);
    let tag = |raw: u32| -> io::Result<String> {
        let slot = slots
            .get(&raw)
            .ok_or_else(|| invalid("War-goal country reference is unresolved"))?;
        String::from_utf8(
            tags.get(slot)
                .ok_or_else(|| invalid("War-goal country tag is missing"))?
                .to_vec(),
        )
        .map_err(|_| invalid("War-goal country tag is not UTF-8"))
    };
    let literals: BTreeMap<_, _> = frame
        .complete::<tables::LexicalTokens>()?
        .iter()
        .map(|r| (r.token, r.literal.as_slice()))
        .collect();
    let goals = frame.complete::<tables::CountryWargoals>()?;
    let justifications = frame.complete::<tables::WargoalJustifications>()?;
    let mut targets = BTreeMap::<_, Vec<_>>::new();
    for row in frame
        .complete::<tables::WargoalTargets>()?
        .iter()
        .filter(|r| same(r.country))
    {
        targets.entry((row.kind, row.goal)).or_default().push(row);
    }
    for rows in targets.values_mut() {
        rows.sort_by_key(|r| r.ordinal);
        if rows
            .iter()
            .enumerate()
            .any(|(i, r)| i != r.ordinal as usize)
        {
            return Err(invalid("War-goal target list is incomplete"));
        }
    }
    let values = |kind, goal| -> Vec<u64> {
        targets
            .get(&(kind, goal))
            .into_iter()
            .flatten()
            .map(|r| r.value)
            .collect()
    };
    let unused: BTreeSet<_> = values(3, 0).into_iter().collect();
    let mut warnings = Vec::new();
    for (kind, goal) in targets.keys() {
        let parent = match kind {
            0..=2 => goals.iter().any(|r| same(r.country) && r.ordinal == *goal),
            3 => *goal == 0,
            4 => justifications
                .iter()
                .any(|r| same(r.country) && r.ordinal == *goal),
            _ => false,
        };
        if !parent {
            return Err(invalid("War-goal targets have no inventory owner"));
        }
    }
    let missing_unused = unused
        .iter()
        .filter(|handle| {
            !goals
                .iter()
                .any(|r| same(r.country) && r.handle == **handle)
        })
        .count();
    if missing_unused != 0 {
        warnings.push(format!("Unused war-goal references refused ({missing_unused}): referenced goal objects are not recorded."));
    }
    let mut goal_name = |token, actor, recipient| -> (String, Option<WargoalTerms>) {
        match literals
            .get(&token)
            .ok_or_else(|| invalid("War-goal type spelling not recorded"))
            .and_then(|literal| name(literal, actor, recipient))
        {
            Ok(name) => name,
            Err(why) => {
                warnings.push(format!("War-goal name refused (1): {why}"));
                ("Not recorded".into(), None)
            }
        }
    };
    let mut out = Vec::new();
    for row in goals.iter().filter(|r| same(r.country)) {
        let (name, terms) = goal_name(row.goal_type, row.actor, row.recipient);
        out.push(WargoalEntry {
            handle: row.handle.to_string(),
            name,
            actor: tag(row.actor)?,
            recipient: tag(row.recipient)?,
            available: unused.contains(&row.handle),
            expires: row.serialized_expiry(),
            states: values(0, row.ordinal)
                .into_iter()
                .map(|v| v as u32)
                .collect(),
            liberate: values(1, row.ordinal)
                .into_iter()
                .map(|v| tag(v as u32))
                .collect::<io::Result<_>>()?,
            puppets: values(2, row.ordinal)
                .into_iter()
                .map(|v| tag(v as u32))
                .collect::<io::Result<_>>()?,
            terms,
        });
    }
    let mut just = Vec::new();
    for row in justifications.iter().filter(|r| same(r.country)) {
        let (name, terms) = goal_name(row.goal_type, row.actor, row.recipient);
        just.push(WargoalJustification {
            ordinal: row.ordinal,
            name,
            actor: tag(row.actor)?,
            recipient: tag(row.recipient)?,
            states: values(4, row.ordinal)
                .into_iter()
                .map(|v| v as u32)
                .collect(),
            total: row.total_cost_raw.to_string(),
            points: row.progress_raw.to_string(),
            daily: row.daily_cost_raw.to_string(),
            pay_upfront: row.pay_upfront != 0,
            start: (row.start != replay_data::rows::WargoalJustificationRow::INVALID_HOUR)
                .then_some(row.start),
            completion: row.completion_hour(p.hour),
            threat: row.threat_raw.to_string(),
            paused: if row.pay_upfront != 0 || row.daily_cost_raw <= 0 {
                Some(false)
            } else {
                selected
                    .political_power_raw
                    .map(|pp| pp < row.daily_cost_raw)
            },
            terms,
        });
    }
    let upfront = just.iter().filter(|j| j.pay_upfront).count();
    if upfront != 0 {
        warnings.push(format!("Upfront political-power amount refused ({upfront}): the activity does not retain its historical payment amount."));
    }
    let unknown_payment = just.iter().filter(|j| j.paused.is_none()).count();
    if unknown_payment != 0 {
        warnings.push(format!("Justification payment status refused ({unknown_payment}): current political power is not recorded."));
    }
    let pairs = out
        .iter()
        .map(|r| (r.actor.clone(), r.recipient.clone()))
        .chain(just.iter().map(|r| (r.actor.clone(), r.recipient.clone())))
        .collect::<BTreeSet<_>>();
    let eligibility = eligibility(frame, &pairs, &mut warnings);
    Ok(WargoalsReply {
        branch_id: p.branch_id.clone(),
        hour: p.hour,
        tag: p.tag.clone(),
        eligibility,
        goals: out,
        justifications: just,
        warnings,
        state_names: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{self, TableRows};
    use replay_frame::frame::TableView;
    #[test]
    fn wargoals_show_all_targets_and_use_unused_membership_instead_of_expiry() {
        let mut frame = super::super::wars::test_fixtures::recorded(100);
        let handle = (8u64 << 32) | 120;
        frame.tables[TableId::CountryWargoals.index()] =
            TableView::Complete(TableRows::CountryWargoals(vec![rows::CountryWargoalRow {
                country: 1,
                ordinal: 0,
                handle,
                actor: 1,
                recipient: 2,
                goal_type: 42,
                name_token: 42,
                expires: 43820000,
            }]))
            .into();
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
                token: 42,
                literal: b"custom_goal".to_vec(),
            }]))
            .into();
        let target = |kind, ordinal, value| rows::WargoalTargetRow {
            country: 1,
            kind,
            goal: 0,
            ordinal,
            value,
        };
        frame.tables[TableId::WargoalTargets.index()] =
            TableView::Complete(TableRows::WargoalTargets(vec![
                target(0, 0, 50),
                target(0, 1, 51),
                target(0, 2, 50),
                target(1, 0, 3),
                target(2, 0, 2),
                target(3, 0, handle),
                target(4, 0, 51),
            ]))
            .into();
        frame.tables[TableId::WargoalJustifications.index()] =
            TableView::Complete(TableRows::WargoalJustifications(vec![
                rows::WargoalJustificationRow {
                    country: 1,
                    actor: 1,
                    goal_type: 42,
                    recipient: 2,
                    total_cost_raw: 9000000,
                    daily_cost_raw: 100000,
                    progress_raw: 1750000,
                    end: 43791240,
                    pay_upfront: 1,
                    ..Default::default()
                },
            ]))
            .into();
        let p = WargoalsParams {
            branch_id: "1".into(),
            hour: 43820001,
            tag: "ONE".into(),
        };
        let terms = WargoalTerms::from(&installed_content_wargoals::wargoals::Terms {
            expiry_days: 77,
            ..Default::default()
        });
        let reply = inventory(&frame, &p, |name, actor, recipient| {
            assert_eq!((actor, recipient), (1, 2));
            Ok((
                String::from_utf8(name.to_vec()).unwrap(),
                Some(terms.clone()),
            ))
        })
        .unwrap();
        assert!(
            reply.goals[0].available,
            "daily-stale expiry cannot remove native membership"
        );
        assert_eq!(reply.goals[0].states, [50, 51, 50]);
        assert_eq!(reply.goals[0].liberate, ["THREE"]);
        assert_eq!(reply.goals[0].puppets, ["TWO"]);
        assert_eq!(reply.justifications[0].points, "1750000");
        assert_eq!(reply.justifications[0].completion, p.hour + 73 * 24);
        assert_eq!(reply.justifications[0].states, [51]);
        assert_eq!(
            reply.justifications[0].terms.as_ref().unwrap().expiry_days,
            77
        );
        assert_eq!(
            reply.justifications[0].terms.as_ref().unwrap().peace.len(),
            4
        );
        assert!(
            reply
                .warnings
                .iter()
                .any(|w| w.starts_with("Upfront political-power amount refused (1)"))
        );
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(Vec::new())).into();
        let without_names = inventory(&frame, &p, |_, _, _| {
            panic!("a missing spelling must not select another definition")
        })
        .unwrap();
        assert_eq!(without_names.goals[0].states, [50, 51, 50]);
        assert_eq!(without_names.goals[0].name, "Not recorded");
        assert!(
            without_names
                .warnings
                .iter()
                .any(|w| w.starts_with("War-goal name refused (1)"))
        );
        let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
        countries
            .iter_mut()
            .find(|c| c.tag == b"ONE")
            .unwrap()
            .political_power_raw = None;
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries)).into();
        let mut activities = frame
            .complete::<tables::WargoalJustifications>()
            .unwrap()
            .to_vec();
        activities[0].pay_upfront = 0;
        activities[0].daily_cost_raw = 0;
        frame.tables[TableId::WargoalJustifications.index()] =
            TableView::Complete(TableRows::WargoalJustifications(activities.clone())).into();
        let reply = inventory(&frame, &p, |_, _, _| Ok(("name".into(), None))).unwrap();
        assert_eq!(
            reply.justifications[0].paused,
            Some(false),
            "a nonpositive rate does not consult political power"
        );
        activities[0].daily_cost_raw = 100000;
        frame.tables[TableId::WargoalJustifications.index()] =
            TableView::Complete(TableRows::WargoalJustifications(activities)).into();
        let reply = inventory(&frame, &p, |_, _, _| Ok(("name".into(), None))).unwrap();
        assert_eq!(reply.justifications[0].paused, None);
        assert!(
            reply
                .warnings
                .iter()
                .any(|w| w.starts_with("Justification payment status refused (1)"))
        );
        frame.tables[TableId::WargoalTargets.index()] = TableView::Unavailable {
            reason: "missing targets".into(),
        }
        .into();
        assert!(inventory(&frame, &p, |_, _, _| Ok(("name".into(), None))).is_err());
    }
    #[test]
    fn goal_names_substitute_recorded_actor_and_target_names_in_both_orders() {
        let mut frame = super::super::wars::test_fixtures::recorded(100);
        let loc = [
            ("custom_goal", "$ACTOR$ claims $TARGET$"),
            ("ONE", "Actor republic"),
            ("TWO", "Target kingdom"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(
            localised_goal_name(&frame, &loc, b"custom_goal", 1, 2).unwrap(),
            "Actor republic claims Target kingdom"
        );
        assert_eq!(
            localised_goal_name(&frame, &loc, b"custom_goal", 2, 1).unwrap(),
            "Target kingdom claims Actor republic"
        );
        frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
            reason: "missing naming operands".into(),
        }
        .into();
        assert!(localised_goal_name(&frame, &loc, b"custom_goal", 1, 2).is_err());
    }

    #[test]
    fn inventory_pair_eligibility_uses_live_wars_and_nonzero_factions_only() {
        let mut frame = super::super::wars::test_fixtures::recorded(100);
        let pairs = BTreeSet::from([("ONE".into(), "TWO".into()), ("TWO".into(), "THREE".into())]);
        let mut warnings = Vec::new();
        let results = eligibility(&frame, &pairs, &mut warnings);
        assert_eq!(
            (results[0].justify, results[0].declare),
            (Some(false), Some(false))
        );
        assert_eq!((results[1].justify, results[1].declare), (None, None));
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|w| w.contains("TWO -> THREE")));
        assert!(
            results.iter().all(|r| r.actor != "THREE"),
            "no unrelated country-pair expansion"
        );
        // A reversed relation with alias endpoints still establishes the same live war.
        let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
        links.push(rows::CountryLinkRow {
            id: 20,
            canonical_slot: 2,
        });
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links)).into();
        let mut relations = vec![rows::WarOwnedRelationRow {
            owner: 1,
            first_country: 20,
            second_country: 1,
            ..Default::default()
        }];
        frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(relations.clone())).into();
        assert_eq!(
            eligibility(&frame, &pairs, &mut Vec::new())[0].declare,
            Some(false)
        );
        relations[0].retired = 1;
        frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(relations)).into();
        assert_eq!(
            eligibility(&frame, &pairs, &mut Vec::new())[0].justify,
            None
        );
        for id in [0, 91] {
            frame.tables[TableId::FactionMembers.index()] =
                TableView::Complete(TableRows::FactionMembers(vec![
                    rows::FactionMemberRow {
                        country: 1,
                        faction: id,
                        ordinal: 0,
                    },
                    rows::FactionMemberRow {
                        country: 20,
                        faction: id,
                        ordinal: 1,
                    },
                ]))
                .into();
            let result = eligibility(&frame, &pairs, &mut Vec::new());
            assert_eq!(result[0].justify, if id == 0 { None } else { Some(false) });
            assert_eq!(result[0].declare, if id == 0 { None } else { Some(false) });
        }
        for table in [TableId::WarOwnedRelations, TableId::FactionMembers] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "missing eligibility operands".into(),
            }
            .into();
        }
        let result = eligibility(&frame, &pairs, &mut Vec::new());
        assert!(
            result
                .iter()
                .all(|r| r.justify.is_none() && r.declare.is_none())
        );
        assert!(eligibility(&frame, &BTreeSet::new(), &mut Vec::new()).is_empty());
    }

    fn gate_frame() -> DecodedFrame {
        let mut frame = super::super::wars::test_fixtures::recorded(100);
        // Only these rosters are additional inputs to the inventory cases.
        // Other absent tables stay unavailable, including singleton tables.
        for table in [
            TableId::CountryWargoals,
            TableId::WargoalTargets,
            TableId::WargoalJustifications,
        ] {
            frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
        }
        // Native tag resolution checks the stored owned-state count.
        frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                (1..=3)
                    .map(|country| rows::CountryCounterInputRow {
                        country,
                        owned_state_count: 1,
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        frame.tables[TableId::ScriptVariables.index()] =
            TableView::Complete(TableRows::ScriptVariables(
                std::iter::once((0, 0))
                    .chain((1..=3).map(|id| (1, id)))
                    .map(|(scope_kind, scope_id)| rows::ScriptVariablesRow {
                        scope_kind,
                        scope_id,
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        frame
    }

    #[test]
    fn authored_gates_use_shared_descriptions_and_count_unsupported_children() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        use dashboard_base::catalogue::tests::folder_catalogue;
        use dashboard_text::{scripted_text::ScriptedText, triggers::Scope};
        let (cat, _folder) = folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n TRIGGER_FULLFILLED_PREFIX:0 \"[yes]\"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"[no]\"\n TRIGGER_COUNTRY_IS:0 \"Country $NAME$\"\n ONE_DEF:0 \"The actor republic\"\n TWO_DEF:0 \"The target kingdom\"\n",
            ),
        ]);
        let frame = gate_frame();
        let content = cat.scripted_content().unwrap();
        content.set_synthetic_environment(Default::default());
        let text = ScriptedText::new(&frame, &cat.localisation, content, 1);
        let allowed = gate_scopes(1, 2, false);
        let available = gate_scopes(1, 2, true);
        assert_eq!(
            (allowed.this, allowed.root, allowed.from, allowed.prev),
            (
                Scope::Country(1),
                Scope::Country(1),
                Some(Scope::Country(1)),
                Some(Scope::Country(1))
            )
        );
        assert_eq!(
            (
                available.this,
                available.root,
                available.from,
                available.prev
            ),
            (
                Scope::Country(2),
                Scope::Country(1),
                Some(Scope::Country(2)),
                Some(Scope::Country(2))
            )
        );
        let source = installed_content_core::parser::parse(b"tag=ONE tag=TWO").unwrap();
        let mut refusals = BTreeMap::new();
        let description = describe_gate(
            &source.items,
            &text,
            &cat.localisation,
            allowed,
            &mut refusals,
        )
        .unwrap();
        assert!(refusals.is_empty(), "{refusals:?}");
        // The flag is omitted from plain text; its separating space remains.
        assert_eq!(
            description,
            "[yes]Country  The actor republic\n[no]Country  The target kingdom\n"
        );
        // Unregistered keys are skipped by the shared native trigger reader.
        let source = installed_content_core::parser::parse(b"unregistered_mod_gate=yes").unwrap();
        assert!(
            describe_gate(
                &source.items,
                &text,
                &cat.localisation,
                gate_scopes(1, 2, false),
                &mut refusals,
            )
            .unwrap()
            .is_empty()
        );
        assert!(refusals.is_empty());
        // These recognized children have no supported description.
        let source = installed_content_core::parser::parse(b"always=yes always=no").unwrap();
        describe_gate(
            &source.items,
            &text,
            &cat.localisation,
            available,
            &mut refusals,
        )
        .unwrap();
        assert_eq!(refusals.values().sum::<usize>(), 2);
        assert!(refusals.keys().all(|why| why.contains("always")));
        refusals.clear();
        let source = installed_content_core::parser::parse(b"tag=THREE").unwrap();
        let description = describe_gate(
            &source.items,
            &text,
            &cat.localisation,
            gate_scopes(3, 2, false),
            &mut refusals,
        )
        .unwrap();
        assert!(description.is_empty());
        assert_eq!(refusals.values().sum::<usize>(), 1);
        assert!(refusals.keys().all(|why| why.contains("THREE_DEF")));
    }
    #[test]
    fn wargoal_product_loads_names_gates_and_eligibility_from_selected_tables() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (cat, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
            ),
            (
                "common/wargoals/test.txt",
                "wargoal_types={Custom_Goal={allowed={tag=ONE} available={tag=TWO always=yes always=no}}}",
            ),
            (
                "localisation/test_l_english.yml",
                "\u{feff}l_english:\n Custom_Goal:0 \"$ACTOR$ claims $TARGET$\"\n ONE:0 \"Actor republic\"\n TWO:0 \"Target kingdom\"\n TRIGGER_FULLFILLED_PREFIX:0 \"[yes]\"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"[no]\"\n TRIGGER_COUNTRY_IS:0 \"Country $NAME$\"\n ONE_DEF:0 \"The actor republic\"\n TWO_DEF:0 \"The target kingdom\"\n",
            ),
        ]);
        cat.scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        let mut frame = gate_frame();
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
                token: 42,
                literal: b"CUSTOM_GOAL".to_vec(),
            }]))
            .into();
        frame.tables[TableId::WargoalJustifications.index()] =
            TableView::Complete(TableRows::WargoalJustifications(vec![
                rows::WargoalJustificationRow {
                    country: 1,
                    actor: 1,
                    recipient: 2,
                    goal_type: 42,
                    ..Default::default()
                },
            ]))
            .into();
        let (source, context) =
            dashboard_base::test_fixtures::source_with_content(vec![frame], &folder.0);
        dashboard_base::catalogue::set_test_catalogue(
            &source,
            context.content.as_ref().unwrap(),
            std::sync::Arc::new(cat),
        );
        let reply = Wargoals::load(
            &source,
            &WargoalsParams {
                branch_id: "1".into(),
                hour: 100,
                tag: "ONE".into(),
            },
            &context,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            reply.justifications[0].name,
            "Actor republic claims Target kingdom"
        );
        let terms = reply.justifications[0].terms.as_ref().unwrap();
        assert!(
            terms
                .allowed
                .as_ref()
                .unwrap()
                .contains("Country  The actor republic")
        );
        assert!(
            terms
                .available
                .as_ref()
                .unwrap()
                .contains("Country  The target kingdom")
        );
        assert!(reply.warnings.iter().any(|why| {
            why.starts_with("War-goal available description refused (2)") && why.contains("always")
        }));
        assert_eq!(reply.eligibility.len(), 1);
        assert_eq!(
            (reply.eligibility[0].justify, reply.eligibility[0].declare),
            (Some(false), Some(false))
        );
        assert!(
            !reply
                .warnings
                .iter()
                .any(|why| why.contains("eligibility refused"))
        );
    }
}
