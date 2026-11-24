use dashboard_base::products::{LoadContext, at_hour::{At, AtHour}};
use dashboard_base::Source;
use super::wargoal_dates::{self, Incarnation, Incarnations};
use dashboard_follow::live::Source as Cell;
use dashboard_base::{
    catalogue::{self, Catalogue, View},
    products::{Country, check_reply_budget, validate_country},
};
use dashboard_text::metadata::{TaggedCountry, tagged_country};
use replay_data::rows::{self, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct Wargoals;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WargoalsParams {
    pub branch_id: String,
    pub country: Country,
}
struct WargoalsAsk {
    branch_id: String,
    hour: i32,
    tag: String,
    country: Country,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
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
    pub state: WargoalState,
    pub granted: Option<i32>,
    pub granted_missing: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WargoalRemoved {
    pub handle: String,
    pub name: Option<String>,
    pub recipient: Option<String>,
    #[doc = " The last hour the goal was witnessed held: its target is named as it was then."]
    pub seen: i32,
    pub granted: Option<i32>,
    pub granted_missing: Option<String>,
    pub removed: i32,
    pub removed_exact: bool,
    pub reason_missing: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WargoalState {
    AtWar,
    Ready,
    InUse,
    Expired,
    Unknown,
}
pub fn goal_state(at_war: Option<bool>, available: bool, expires: Option<i32>, hour: i32) -> WargoalState {
    let Some(at_war) = at_war else { return WargoalState::Unknown };
    if at_war {
        WargoalState::AtWar
    } else if expires.is_some_and(|expires| expires <= hour) {
        WargoalState::Expired
    } else if available {
        WargoalState::Ready
    } else {
        WargoalState::InUse
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WargoalJustification {
    #[doc = " Its row key: the row of its live cell `points`, the stored fixed5 progress."]
    pub id: String,
    pub ordinal: u32,
    pub name: String,
    pub actor: String,
    pub recipient: String,
    pub states: Vec<u32>,
    pub total: String,
    pub daily: String,
    pub pay_upfront: bool,
    pub start: Option<i32>,
    pub completion: i32,
    pub paused: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WargoalStateName {
    pub id: u32,
    pub name: String,
    pub known: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WargoalPair {
    pub actor: String,
    pub recipient: String,
    #[doc = " What the recorded wars and factions prove about the pair; none when neither applies."]
    pub standing: Option<WargoalStanding>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WargoalStanding {
    AtWar,
    SameFaction,
    WarUnread,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WargoalsReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    pub standings: Vec<WargoalPair>,
    pub goals: Vec<WargoalEntry>,
    pub justifications: Vec<WargoalJustification>,
    pub removed: Vec<WargoalRemoved>,
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

fn read_tables() -> Vec<TableId> {
    let mut tables = TABLES.to_vec();
    tables.extend(content_reader_country_names::country_names::TABLES);
    tables.sort_unstable();
    tables.dedup();
    tables
}
dashboard_base::reply_warnings!(WargoalsReply);

pub struct WargoalContent {
    catalogue: Result<Arc<Catalogue>, String>,
    dates: Result<Arc<Incarnations>, String>,
}

impl AtHour for Wargoals {
    const KEY: &'static str = "wargoals";
    const TITLE: &'static str = "War goals";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::EveryHour("computes justification completion hours and wargoal available/expiry state against at.hour");
    type Params = WargoalsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = WargoalsReply;
    type Context = WargoalContent;

    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        validate_country(&p.country)?;
        p.branch_id.parse::<u64>().map_err(|_| "Invalid war-goal branch")?;
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            WargoalsParams::decl(),
            WargoalEntry::decl(),
            WargoalJustification::decl(),
            WargoalsReply::decl(),
            WargoalPair::decl(),
            WargoalStanding::decl(),
            WargoalState::decl(),
            WargoalStateName::decl(),
            WargoalRemoved::decl(),
        ]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        read_tables()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| why.to_string());
        Ok(WargoalContent { catalogue, dates: Err("War-goal dates need the branch's history".into()) })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| why.to_string());
        let dates = params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch".to_owned())
            .and_then(|branch| wargoal_dates::follow(source, branch).map_err(|why| format!("War-goal dates refused: {why}")));
        Ok(WargoalContent { catalogue, dates })
    }

    fn reply(frame: &DecodedFrame, p: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("War goals country is unavailable at this hour"))?;
        let tag_at = |slot: u32, hour: i32| at.holders.tag_at(&Country::Slot(slot), hour).map(str::to_owned);
        let reply = reply(frame, &WargoalsAsk { branch_id: p.branch_id.clone(), hour: at.hour, tag: at.holders.name(&p.country, at.hour), country: p.country.clone() }, owner.slot, &at.context.catalogue, &at.context.dates, &tag_at)?;
        let progress = dashboard_declare::field(TableId::WargoalJustifications, "progress_raw")?;
        for justification in &reply.justifications {
            let key = justification.id.parse::<u128>().map_err(|_| invalid("War-goal justification key is not a number"))?;
            at.live.value(&justification.id, "points", Cell { table: TableId::WargoalJustifications, column: progress, key }, 1.0);
        }
        Ok(reply)
    }
}

fn reply(
    frame: &DecodedFrame,
    p: &WargoalsAsk,
    slot: u32,
    catalogue: &Result<Arc<Catalogue>, String>,
    dates: &Result<Arc<Incarnations>, String>,
    tag_at: &dyn Fn(u32, i32) -> Option<String>,
) -> io::Result<WargoalsReply> {
    let held = catalogue.as_ref().ok().map(|cat| {
        cat.content
            .layers
            .derived::<content_reader_layers::content::diagnostics::Once<Result<content_reader_wargoals::wargoals::Database, String>>>()
    });
    let definitions = match (catalogue.as_ref(), held.as_ref()) {
        (Ok(cat), Some(held)) => held
            .get_or_init(|| content_reader_wargoals::wargoals::read(&cat.content.layers).map_err(|why| why.to_string()))
            .as_ref()
            .map_err(Clone::clone),
        (Err(why), _) => Err(why.clone()),
        _ => Err("War-goal content is unavailable".into()),
    };
    let text = catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
        cat.scripted_content()
            .map(|scripted| dashboard_text::scripted_text::ScriptedText::new(frame, &cat.localisation, scripted, slot))
    });
    let links: BTreeMap<_, _> = frame.complete::<tables::CountryLinks>()?.iter().map(|r| (r.id, r.canonical_slot)).collect();
    let mut name_warnings = Vec::new();
    let mut name_of = |literal: &[u8], actor: u32, recipient: u32| -> io::Result<String> {
        let slots = || -> Result<(u32, u32), String> {
            Ok((*links.get(&actor).ok_or("War-goal actor is unresolved")?, *links.get(&recipient).ok_or("War-goal target is unresolved")?))
        };
        let definition_name = definitions
            .as_ref()
            .map_err(|why| invalid(why.clone()))?
            .find(literal)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("War-goal definition is absent from the recorded content"))?;
        let label = (|| {
            let (actor, recipient) = slots()?;
            let cat = catalogue.as_ref().map_err(Clone::clone)?;
            localised_goal_name(frame, &cat.localisation, definition_name, actor, recipient)
        })();
        Ok(match label {
            Ok(name) => name,
            Err(why) => {
                name_warnings.push(format!("War-goal name refused (1): {why}"));
                "Not recorded".into()
            }
        })
    };
    let mut reply = inventory(frame, p, &mut name_of)?;
    let literals: BTreeMap<_, _> = frame.complete::<tables::LexicalTokens>()?.iter().map(|r| (r.token, r.literal.clone())).collect();
    let owners: Vec<u32> = links.iter().filter(|(_, held)| **held == slot).map(|(raw, _)| *raw).collect();
    match dates {
        Ok(dates) => apply_dates(&mut reply, dates, p.hour, &owners, |incarnation| {
            let token = incarnation.goal_type.and_then(|token| u32::try_from(token).ok());
            let actor = incarnation.actor.and_then(|raw| u32::try_from(raw).ok());
            let recipient = incarnation.recipient.and_then(|raw| u32::try_from(raw).ok());
            let name = match (token.and_then(|token| literals.get(&token)), actor, recipient) {
                (Some(literal), Some(actor), Some(recipient)) => name_of(literal, actor, recipient).ok().filter(|name| name != "Not recorded"),
                _ => None,
            };
            (name, recipient.and_then(|raw| links.get(&raw)).and_then(|slot| tag_at(*slot, incarnation.last_seen)))
        }),
        Err(why) => {
            for goal in &mut reply.goals {
                goal.granted_missing = Some(why.clone());
            }
            reply.warnings.push(why.clone());
        }
    }
    reply.warnings.extend(name_warnings);
    let ids: BTreeSet<_> = reply
        .goals
        .iter()
        .flat_map(|g| g.states.iter())
        .chain(reply.justifications.iter().flat_map(|j| j.states.iter()))
        .copied()
        .collect();
    for id in ids {
        let result = text.as_ref().map_err(Clone::clone).and_then(|text| text.plain_state_name(id));
        let (name, known) = match result {
            Ok(name) => (name, true),
            Err(why) => {
                reply.warnings.push(format!("War-goal state name refused (1): {why}"));
                ("Not recorded".into(), false)
            }
        };
        reply.state_names.push(WargoalStateName { id, name, known });
    }
    if let Ok(definitions) = definitions {
        reply.warnings.extend(definitions.notes.iter().cloned());
    }
    check_reply_budget(&reply)?;
    Ok(reply)
}

pub(crate) fn apply_dates(
    reply: &mut WargoalsReply,
    dates: &Incarnations,
    hour: i32,
    owners: &[u32],
    mut describe: impl FnMut(&Incarnation) -> (Option<String>, Option<String>),
) {
    let held_at = |incarnation: &Incarnation| incarnation.granted.is_none_or(|granted| granted <= hour) && incarnation.removed.is_none_or(|removed| removed > hour);
    for goal in &mut reply.goals {
        let found = goal.handle.parse::<u64>().ok().and_then(|handle| dates.of_handle(handle).find(|incarnation| held_at(incarnation)));
        match found {
            Some(incarnation) => {
                goal.granted = incarnation.granted;
                goal.granted_missing = incarnation.granted_missing.map(str::to_owned);
            }
            None => goal.granted_missing = Some(wargoal_dates::GRANT_NOT_READ.into()),
        }
    }
    let mut removed: Vec<WargoalRemoved> = owners
        .iter()
        .flat_map(|owner| dates.of_country(*owner))
        .filter(|incarnation| incarnation.removed.is_some_and(|removed| removed <= hour))
        .map(|incarnation| {
            let (name, recipient) = describe(incarnation);
            WargoalRemoved {
                handle: incarnation.handle.to_string(),
                name,
                recipient,
                seen: incarnation.last_seen,
                granted: incarnation.granted,
                granted_missing: incarnation.granted_missing.map(str::to_owned),
                removed: incarnation.removed.unwrap_or(hour),
                removed_exact: incarnation.removed_exact,
                reason_missing: wargoal_dates::REMOVAL_REASON.to_owned(),
            }
        })
        .collect();
    removed.sort_by_key(|item| (std::cmp::Reverse(item.removed), item.handle.clone()));
    reply.removed = removed;
}

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}
fn selected_country<'a>(
    countries: &'a [rows::CountryRow],
    links: &[rows::CountryLinkRow],
    country: &Country,
) -> io::Result<(&'a rows::CountryRow, u32)> {
    match dashboard_base::products::country_in_frame(countries, links, country) {
        TaggedCountry::Found(country, slot) => Ok((country, slot)),
        TaggedCountry::Absent => Err(invalid("War-goal country is absent at this hour")),
        TaggedCountry::Unresolved => Err(invalid(
            "War-goal country identity is unresolved at this hour",
        )),
    }
}
fn localised_goal_name(
    frame: &DecodedFrame,
    localisation: &content_reader_localisation::localisation::Localisation,
    definition: &[u8],
    actor: u32,
    recipient: u32,
) -> Result<String, String> {
    let names = content_reader_country_names::country_names::CountryNames::read_selected(
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
                content_reader_localisation::localisation::Argument::Text(&actor),
            ),
            (
                "TARGET",
                content_reader_localisation::localisation::Argument::Text(&target),
            ),
        ],
        View::Plain,
    )
}

fn standings(frame: &DecodedFrame, pairs: &BTreeSet<(String, String)>) -> Vec<WargoalPair> {
    let links = frame.complete::<tables::CountryLinks>().ok();
    let countries = frame.complete::<tables::Countries>().ok();
    let slot = |tag: &str| {
        tagged_country(countries?, links?, tag)
            .found()
            .map(|(_, slot)| slot)
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
            let war = known.zip(relations.as_ref()).map(|((actor, recipient), rows)| {
                {
                    rows.iter().any(|r| {
                        r.retired == 0
                            && canonical(r.owner) == Some(actor)
                            && ((canonical(r.first_country) == Some(actor)
                                && canonical(r.second_country) == Some(recipient))
                                || (canonical(r.second_country) == Some(actor)
                                    && canonical(r.first_country) == Some(recipient)))
                    })
                }
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
            WargoalPair {
                actor: actor.clone(),
                recipient: recipient.clone(),
                standing: match war {
                    Some(true) => Some(WargoalStanding::AtWar),
                    None => Some(WargoalStanding::WarUnread),
                    Some(false) => same_faction.then_some(WargoalStanding::SameFaction),
                },
            }
        })
        .collect()
}

fn inventory(
    frame: &DecodedFrame,
    p: &WargoalsAsk,
    mut name: impl FnMut(&[u8], u32, u32) -> io::Result<String>,
) -> io::Result<WargoalsReply> {
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let (selected, selected_slot) = selected_country(countries, links, &p.country)?;
    let slots: BTreeMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
    let tags: BTreeMap<_, _> = countries
        .iter()
        .map(|r| (r.canonical_slot, r.tag.as_slice()))
        .collect();
    let same = |raw: u32| slots.get(&raw) == Some(&selected_slot);
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
    let mut goal_name = |token, actor, recipient| -> String {
        match literals
            .get(&token)
            .ok_or_else(|| dashboard_core::refused("War-goal type spelling not recorded"))
            .and_then(|literal| name(literal, actor, recipient))
        {
            Ok(name) => name,
            Err(why) => {
                warnings.push(format!("War-goal name refused (1): {why}"));
                "Not recorded".into()
            }
        }
    };
    let mut out = Vec::new();
    for row in goals.iter().filter(|r| same(r.country)) {
        let name = goal_name(row.goal_type, row.actor, row.recipient);
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
            state: WargoalState::InUse,
            granted: None,
            granted_missing: None,
        });
    }
    let mut just = Vec::new();
    for row in justifications.iter().filter(|r| same(r.country)) {
        let name = goal_name(row.goal_type, row.actor, row.recipient);
        just.push(WargoalJustification {
            id: (u128::from(row.country) << 32 | u128::from(row.ordinal)).to_string(),
            ordinal: row.ordinal,
            name,
            actor: tag(row.actor)?,
            recipient: tag(row.recipient)?,
            states: values(4, row.ordinal)
                .into_iter()
                .map(|v| v as u32)
                .collect(),
            total: row.total_cost_raw.to_string(),
            daily: row.daily_cost_raw.to_string(),
            pay_upfront: row.pay_upfront != 0,
            start: (row.start != replay_data::rows::WargoalJustificationRow::INVALID_HOUR)
                .then_some(row.start),
            completion: row.completion_hour(p.hour),
            paused: if row.pay_upfront != 0 || row.daily_cost_raw <= 0 {
                Some(false)
            } else {
                selected
                    .political_power_raw
                    .map(|pp| pp < row.daily_cost_raw)
            },
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
    let standings = standings(frame, &pairs);
    let war_of: BTreeMap<(&str, &str), Option<bool>> = standings
        .iter()
        .map(|pair| ((pair.actor.as_str(), pair.recipient.as_str()), match pair.standing {
            Some(WargoalStanding::AtWar) => Some(true),
            Some(WargoalStanding::WarUnread) => None,
            _ => Some(false),
        }))
        .collect();
    for goal in &mut out {
        let at_war = war_of.get(&(goal.actor.as_str(), goal.recipient.as_str())).copied().unwrap_or(None);
        goal.state = goal_state(at_war, goal.available, goal.expires, p.hour);
    }
    out.sort_by_key(|goal| goal.state);
    Ok(WargoalsReply {
        branch_id: p.branch_id.clone(),
        tag: p.tag.clone(),
        asked: p.country.clone(),
        standings,
        goals: out,
        justifications: just,
        removed: Vec::new(),
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
        let mut frame = super::super::war_fixtures::recorded(100);
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
        let p = WargoalsAsk {
            branch_id: "1".into(),
            hour: 43820001,
            tag: "ONE".into(),
            country: "ONE".into(),
        };
        let reply = inventory(&frame, &p, |name, actor, recipient| {
            assert_eq!((actor, recipient), (1, 2));
            Ok(String::from_utf8(name.to_vec()).unwrap())
        })
        .unwrap();
        assert!(
            reply.goals[0].available,
            "daily-stale expiry cannot remove native membership"
        );
        assert_eq!(reply.goals[0].state, WargoalState::AtWar, "a goal against a country at war is at war, its expiry passed or not");
        assert_eq!(reply.goals[0].states, [50, 51, 50]);
        assert_eq!(reply.goals[0].liberate, ["THREE"]);
        assert_eq!(reply.goals[0].puppets, ["TWO"]);
        assert_eq!(reply.justifications[0].id, (1u128 << 32).to_string(), "the row its live progress is read from");
        assert_eq!(reply.justifications[0].completion, p.hour + 73 * 24);
        assert_eq!(reply.justifications[0].states, [51]);
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
        let reply = inventory(&frame, &p, |_, _, _| Ok("name".into())).unwrap();
        assert_eq!(
            reply.justifications[0].paused,
            Some(false),
            "a nonpositive rate does not consult political power"
        );
        activities[0].daily_cost_raw = 100000;
        frame.tables[TableId::WargoalJustifications.index()] =
            TableView::Complete(TableRows::WargoalJustifications(activities)).into();
        let reply = inventory(&frame, &p, |_, _, _| Ok("name".into())).unwrap();
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
        assert!(inventory(&frame, &p, |_, _, _| Ok("name".into())).is_err());
    }
    #[test]
    fn goal_names_substitute_recorded_actor_and_target_names_in_both_orders() {
        let mut frame = super::super::war_fixtures::recorded(100);
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
    fn a_goal_is_followed_by_its_handle_across_shifting_rows_and_dated_only_where_read() {
        use replay_lanes::lanes::CoveredValue::{Unvalued, Value};
        let row = |ordinal: u128| (1u128 << 32) | ordinal;
        let handles = std::collections::HashMap::from([
            (row(0), vec![(100, 109, Value(7)), (110, 111, Unvalued), (112, 130, Value(9))]),
            (row(1), vec![(100, 109, Unvalued), (110, 120, Value(7)), (121, 130, Unvalued)]),
        ]);
        let attributes: [std::collections::HashMap<u128, replay_lanes::lanes::Runs>; 4] = [
            std::collections::HashMap::from([(row(1), vec![(100, 130, Some(1))])]),
            std::collections::HashMap::from([(row(1), vec![(100, 130, Some(2))])]),
            std::collections::HashMap::from([(row(1), vec![(100, 130, Some(42))])]),
            std::collections::HashMap::new(),
        ];
        let found = wargoal_dates::Incarnations::new(wargoal_dates::incarnations(&handles, &attributes, Some(100), 130));
        let seven = found.of_handle(7).next().unwrap();
        assert_eq!((seven.granted, seven.removed, seven.removed_exact, seven.country), (None, Some(121), true, 1), "held at the first read: granted before the branch; removed the hour it was first read gone");
        assert_eq!((seven.recipient, seven.goal_type), (Some(2), Some(42)), "its terms are read from the row that held it last");
        let nine = found.of_handle(9).next().unwrap();
        assert_eq!((nine.granted, nine.removed), (Some(112), None), "still held at the branch's last read");
        let mut reply = WargoalsReply {
            branch_id: "1".into(), tag: "ONE".into(), asked: "ONE".into(), standings: Vec::new(),
            goals: vec![WargoalEntry { handle: "9".into(), name: "Conquer".into(), actor: "ONE".into(), recipient: "TWO".into(), available: true, expires: None, states: Vec::new(), liberate: Vec::new(), puppets: Vec::new(), state: WargoalState::Ready, granted: None, granted_missing: None }],
            justifications: Vec::new(), removed: Vec::new(), warnings: Vec::new(), state_names: Vec::new(),
        };
        apply_dates(&mut reply, &found, 120, &[1], |_| (Some("Conquer".into()), Some("TWO".into())));
        assert_eq!(reply.goals[0].granted, Some(112));
        assert!(reply.removed.is_empty(), "a removal after the displayed hour is not shown");
        apply_dates(&mut reply, &found, 125, &[1], |_| (Some("Conquer".into()), Some("TWO".into())));
        assert_eq!(reply.removed.len(), 1);
        assert_eq!((reply.removed[0].removed, reply.removed[0].granted_missing.as_deref()), (121, Some(wargoal_dates::HELD_AT_BRANCH_START)));
        assert_eq!(reply.removed[0].reason_missing, wargoal_dates::REMOVAL_REASON);
        assert_eq!(reply.removed[0].seen, 120, "the target is named as it was the last hour the goal was seen");
    }

    #[test]
    fn a_removal_is_dated_by_its_first_witnessed_absence_and_a_grant_after_a_gap_says_so() {
        use replay_lanes::lanes::CoveredValue::{Uncovered, Unvalued, Value};
        let row = |ordinal: u128| (1u128 << 32) | ordinal;
        let handles = std::collections::HashMap::from([
            (row(0), vec![(100, 109, Value(7)), (110, 119, Uncovered), (120, 130, Unvalued)]),
            (row(1), vec![(100, 104, Unvalued), (105, 107, Uncovered), (108, 130, Value(8))]),
            (row(2), vec![(100, 112, Value(5)), (113, 130, Uncovered)]),
        ]);
        let none: [std::collections::HashMap<u128, replay_lanes::lanes::Runs>; 4] = Default::default();
        let found = wargoal_dates::Incarnations::new(wargoal_dates::incarnations(&handles, &none, Some(100), 130));
        let seven = found.of_handle(7).next().unwrap();
        assert_eq!((seven.removed, seven.removed_exact, seven.last_seen), (Some(120), false, 109), "removed by the first hour it was read gone, not the hour after it was last seen");
        assert_eq!(seven.granted_missing, Some(wargoal_dates::HELD_AT_BRANCH_START));
        let eight = found.of_handle(8).next().unwrap();
        assert_eq!((eight.granted, eight.granted_missing), (None, Some(wargoal_dates::FIRST_SEEN_AFTER_GAP)), "first seen after a gap is not held at branch start");
        let five = found.of_handle(5).next().unwrap();
        assert_eq!(five.removed, None, "never read gone is never removed");
        assert_eq!(found.of_country(1).count(), 3);
    }
    #[test]
    fn a_goal_state_puts_war_before_readiness_and_expiry_before_both_unless_at_war() {
        assert_eq!(goal_state(Some(true), true, Some(10), 20), WargoalState::AtWar);
        assert_eq!(goal_state(None, true, Some(10), 20), WargoalState::Unknown, "unread war relations prove neither war nor peace");
        assert_eq!(goal_state(Some(false), true, Some(20), 20), WargoalState::Expired, "a goal is expired from its expiry hour");
        assert_eq!(goal_state(Some(false), true, Some(21), 20), WargoalState::Ready);
        assert_eq!(goal_state(Some(false), true, None, 20), WargoalState::Ready, "no recorded expiry is not an expiry");
        assert_eq!(goal_state(Some(false), false, None, 20), WargoalState::InUse);
        let mut states = [WargoalState::Unknown, WargoalState::Expired, WargoalState::InUse, WargoalState::Ready, WargoalState::AtWar];
        states.sort();
        assert_eq!(states, [WargoalState::AtWar, WargoalState::Ready, WargoalState::InUse, WargoalState::Expired, WargoalState::Unknown]);
    }
    #[test]
    fn inventory_pair_standing_uses_live_wars_and_nonzero_factions_only() {
        let mut frame = super::super::war_fixtures::recorded(100);
        let pairs = BTreeSet::from([("ONE".into(), "TWO".into()), ("TWO".into(), "THREE".into())]);
        let results = standings(&frame, &pairs);
        assert_eq!(results[0].standing, Some(WargoalStanding::AtWar));
        assert_eq!(results[1].standing, None);
        assert!(
            results.iter().all(|r| r.actor != "THREE"),
            "no unrelated country-pair expansion"
        );
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
            standings(&frame, &pairs)[0].standing,
            Some(WargoalStanding::AtWar)
        );
        relations[0].retired = 1;
        frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(relations)).into();
        assert_eq!(standings(&frame, &pairs)[0].standing, None);
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
            let result = standings(&frame, &pairs);
            assert_eq!(
                result[0].standing,
                (id != 0).then_some(WargoalStanding::SameFaction)
            );
        }
        for table in [TableId::WarOwnedRelations, TableId::FactionMembers] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "missing standing operands".into(),
            }
            .into();
        }
        let result = standings(&frame, &pairs);
        assert!(result.iter().all(|r| r.standing == Some(WargoalStanding::WarUnread)), "unread war relations prove neither war nor peace");
        assert!(standings(&frame, &BTreeSet::new()).is_empty());
    }

    fn gate_frame() -> DecodedFrame {
        let mut frame = super::super::war_fixtures::recorded(100);
        for table in [
            TableId::CountryWargoals,
            TableId::WargoalTargets,
            TableId::WargoalJustifications,
        ] {
            frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
        }
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
    fn the_selected_country_is_the_first_link_resolved_holder_of_its_tag() {
        let mut frame = gate_frame();
        let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
        countries.insert(
            0,
            rows::CountryRow {
                id: 50,
                canonical_slot: 50,
                tag: b"ONE".to_vec(),
                ..Default::default()
            },
        );
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries)).into();
        frame.tables[TableId::CountryWargoals.index()] =
            TableView::Complete(TableRows::CountryWargoals(vec![rows::CountryWargoalRow {
                country: 1,
                ordinal: 0,
                handle: 7,
                actor: 1,
                recipient: 2,
                goal_type: 42,
                name_token: 42,
                expires: 43820000,
            }]))
            .into();
        let p = WargoalsAsk {
            branch_id: "1".into(),
            hour: 100,
            tag: "ONE".into(),
            country: "ONE".into(),
        };
        let reply = inventory(&frame, &p, |_, _, _| Ok("name".into())).unwrap();
        assert_eq!(reply.goals.len(), 1);
        assert_eq!(reply.goals[0].actor, "ONE");
        assert_eq!(reply.standings[0].standing, Some(WargoalStanding::AtWar));
        let links: Vec<_> = frame
            .complete::<tables::CountryLinks>()
            .unwrap()
            .iter()
            .filter(|link| link.id != 1)
            .cloned()
            .collect();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links)).into();
        let unresolved = inventory(&frame, &p, |_, _, _| Ok("name".into())).unwrap_err();
        assert!(
            unresolved.to_string().contains("unresolved"),
            "{unresolved}"
        );
    }

    #[test]
    fn wargoal_product_loads_names_and_standings_from_selected_tables() {
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
        let _folder = folder;
        let reply = reply(
            &frame,
            &WargoalsAsk {
                branch_id: "1".into(),
                hour: 100,
                tag: "ONE".into(),
                country: "ONE".into(),
            },
            1,
            &Ok(Arc::new(cat)),
            &Ok(Arc::new(wargoal_dates::Incarnations::new(Vec::new()))),
            &|_, _| None,
        )
        .unwrap();
        assert_eq!(
            reply.justifications[0].name,
            "Actor republic claims Target kingdom"
        );
        assert!(
            reply
                .warnings
                .iter()
                .all(|why| !why.contains("description")),
            "no authored gate is described"
        );
        assert_eq!(reply.standings.len(), 1);
        assert_eq!(reply.standings[0].standing, Some(WargoalStanding::AtWar));
    }
}
