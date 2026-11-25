use super::{Assembled, PeaceChange, PeaceDeal, PeaceObservation, PeaceWarning, assemble, deal_hour, deal_known_at};
use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply},
    },
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::occurrence::{Ask, Chip, Direction, Index, Occurrence, Order, PlacedGap, Role, Selection},
    series::{MissingSpan, ranked::GroupSelection},
};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeaceDealsTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub expanded: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub terms_per_line: Option<u32>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PeaceTermKind {
    State,
    Puppet,
    Government,
    Release,
    Ship,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTermCount {
    pub kind: PeaceTermKind,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PeaceLine {
    Treaty { winner: Option<String>, loser: Option<String>, terms: Vec<PeaceTermCount>, white_peace: bool, members: usize, bounded: bool, derived: bool, limitations: Vec<String> },
    Recipient { tag: Option<String>, terms: Vec<PeaceTermCount> },
    Terms { items: Vec<PeaceTerm> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceTerm {
    pub kind: PeaceTermKind,
    pub state: Option<u32>,
    pub tag: Option<String>,
    pub at: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceDealsRow {
    pub key: String,
    pub parent: Option<String>,
    pub hour: i32,
    pub detail: PeaceLine,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PeaceDealsTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub counts: Vec<OccurrenceCount>,
    pub rows: Vec<PeaceDealsRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PeaceDealsTailReply);

impl OccurrenceReply for PeaceDealsTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }
    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

type Recipients = BTreeMap<Option<String>, Vec<PeaceTerm>>;

fn recipients(deal: &PeaceDeal) -> Recipients {
    use PeaceTermKind::{Government, Other, Puppet, Release, Ship, State};

    let hour = deal_hour(deal);
    let mut recipients = Recipients::new();
    let former_owners: BTreeMap<_, _> = deal
        .items
        .iter()
        .filter_map(|item| match &item.change {
            PeaceChange::StateTransfer { state, from_country, to, .. } if item.derived => Some(((*state, to), from_country)),
            _ => None,
        })
        .collect();
    let mut states = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut add = |recipient: &Option<String>, kind, id: String, state: Option<u32>, tag: Option<String>, at| {
        if state.is_some_and(|id| !states.insert(id)) {
            return;
        }
        if targets.insert((recipient.clone(), kind, id)) {
            recipients.entry(recipient.clone()).or_default().push(PeaceTerm { kind, state, tag, at });
        }
    };
    let mut captured_countries = BTreeSet::new();
    if let Some(captured) = &deal.captured {
        for transfer in &captured.transfers {
            let state = u32::try_from(transfer.state).ok();
            let previous = transfer.to.as_ref().and_then(|to| former_owners.get(&(state?, to))).filter(|_| transfer.from.is_none());
            let tag = transfer.from.clone().or_else(|| previous.map(|c| c.tag.clone()));
            add(&transfer.to, State, transfer.state.to_string(), state, tag, previous.map_or(hour, |c| c.hour));
        }
        for subject in &captured.subjects {
            if let Some(tag) = &subject.subject_tag {
                captured_countries.insert((Puppet, tag));
            }
            add(&subject.overlord_tag, Puppet, subject.subject.to_string(), None, subject.subject_tag.clone(), hour);
        }
        let transferred: BTreeSet<_> = captured.transfers.iter().map(|transfer| transfer.state).collect();
        for action in &captured.actions {
            match action.form {
                0 if action.state.is_some_and(|id| transferred.contains(&id)) => {}
                2 => {}
                1 | 3 => {
                    let kind = if action.form == 1 { Release } else { Government };
                    if let Some(tag) = &action.country_operand_tag {
                        captured_countries.insert((kind, tag));
                    }
                    let id = action.country_operand.map_or_else(|| format!("{}:{}:{}", action.country, action.turn, action.ordinal), |id| id.to_string());
                    add(&action.taker_tag, kind, id, None, action.country_operand_tag.clone(), hour);
                }
                4 => add(
                    &action.taker_tag,
                    Ship,
                    action.ship.clone().unwrap_or_else(|| format!("{}:{}", action.turn, action.ordinal)),
                    None,
                    action.giver_tag.clone(),
                    hour,
                ),
                _ => add(&action.taker_tag, Other, format!("{}:{}:{}", action.country, action.turn, action.ordinal), None, action.giver_tag.clone(), hour),
            }
        }
    }
    for item in &deal.items {
        let at = item.dates.after_hour;
        match &item.change {
            PeaceChange::StateTransfer { state, from_country, .. } => {
                add(&item.winner, State, state.to_string(), Some(*state), Some(from_country.tag.clone()), from_country.hour)
            }
            PeaceChange::NewSubject { subject, .. } => {
                if !captured_countries.contains(&(Puppet, subject)) {
                    add(&item.winner, Puppet, subject.clone(), None, Some(subject.clone()), at);
                }
            }
            PeaceChange::GovernmentChange { country, .. } => {
                if !captured_countries.contains(&(Government, country)) {
                    add(&item.winner, Government, country.clone(), None, Some(country.clone()), at);
                }
            }
            PeaceChange::CountryReleased { country, transfers, .. } => {
                if !captured_countries.contains(&(Release, country)) {
                    add(&item.winner, Release, country.clone(), None, Some(country.clone()), at);
                }
                for transfer in transfers {
                    add(
                        &Some(country.clone()),
                        State,
                        transfer.state.to_string(),
                        Some(transfer.state),
                        Some(transfer.from_country.tag.clone()),
                        transfer.from_country.hour,
                    );
                }
            }
        }
    }
    recipients
}

fn counts(terms: impl Iterator<Item = PeaceTermKind>) -> Vec<PeaceTermCount> {
    let mut totals = BTreeMap::new();
    for term in terms {
        *totals.entry(term).or_default() += 1;
    }
    totals.into_iter().map(|(kind, count)| PeaceTermCount { kind, count }).collect()
}

struct Line {
    row: PeaceDealsRow,
    position: usize,
}

struct Tail {
    index: Index<Line>,
    warnings: Vec<PeaceWarning>,
}

impl Tail {
    fn new(mut assembled: Assembled, holders: &Holders) -> io::Result<Self> {
        let mut entries = Vec::new();
        let mut joined = BTreeMap::<usize, Vec<usize>>::new();
        for (at, outcome) in assembled.deals.iter().enumerate().filter(|(_, d)| d.observation == PeaceObservation::ConferenceNotRecorded) {
            let mut matches = assembled.deals.iter().enumerate().filter(|(_, d)| {
                d.captured.is_some()
                    && deal_hour(d) == deal_hour(outcome)
                    && deal_known_at(d) == deal_known_at(outcome)
                    && !outcome.winners.is_empty()
                    && !outcome.losers.is_empty()
                    && outcome.winners.iter().all(|tag| d.winners.contains(tag))
                    && outcome.losers.iter().all(|tag| d.losers.contains(tag))
            });
            if let Some((conference, _)) = matches.next()
                && matches.next().is_none()
            {
                joined.entry(conference).or_default().push(at);
            }
        }
        let mut joined_rows = BTreeSet::new();
        for (conference, outcomes) in joined {
            for at in outcomes {
                let items = std::mem::take(&mut assembled.deals[at].items);
                assembled.deals[conference].items.extend(items);
                joined_rows.insert(at);
            }
        }
        for (at, deal) in assembled.deals.into_iter().enumerate() {
            if joined_rows.contains(&at) {
                continue;
            }
            let hour = deal_hour(&deal);
            let observed = deal_known_at(&deal);
            if let (Some(why), Some(at)) = (&deal.capture_unavailable, deal.capture_unavailable_hour) {
                assembled.warnings.push(PeaceWarning::at(at, why.clone()));
            }
            let group = format!("group:{}", deal.id);
            let mut groups = recipients(&deal);
            if groups.is_empty()
                && let Some(capture) = &deal.captured
            {
                for winner in &capture.winners {
                    groups.entry(winner.clone()).or_default();
                }
            }
            let mut roles: Vec<_> =
                deal.winners
                    .iter()
                    .chain(&deal.losers)
                    .map(|tag| (tag, hour))
                    .chain(groups.iter().flat_map(|(tag, terms)| {
                        tag.iter().map(|tag| (tag, hour)).chain(terms.iter().filter_map(|term| Some((term.tag.as_ref()?, term.at))))
                    }))
                    .map(|(tag, at)| Role { slot: holders.slot_at(&Country::Tag(tag.clone()), at), at })
                    .collect();
            roles.sort_unstable_by_key(|role| (role.slot, role.at));
            roles.dedup();
            let mut push = |parent: Option<String>, detail, position| {
                let key = if parent.is_none() { group.clone() } else { format!("{group}:{}", entries.len()) };
                entries.push(Occurrence {
                    id: u128::MAX - entries.len() as u128,
                    date: hour,
                    observed,
                    kind: u16::from(parent.is_some()),
                    roles: roles.clone().into_boxed_slice(),
                    subjects: Box::default(),
                    states: Box::default(),
                    sort_keys: Box::default(),
                    text: Box::default(),
                    payload: Line { row: PeaceDealsRow { key, parent, hour, detail }, position },
                });
            };
            let terms = counts(groups.values().flatten().map(|term| term.kind));
            let first_tag = |captured: Option<&[Option<String>]>, derived: &[String]| match captured {
                Some(tags) => tags.first().cloned().flatten(),
                None => derived.first().cloned(),
            };
            let captured = deal.captured.as_ref();
            let white_peace = captured.is_some_and(|c| c.actions.is_empty() && c.transfers.is_empty() && c.subjects.is_empty()) && terms.is_empty();
            let members = groups.values().map(|terms| 1 + terms.len()).sum();
            push(
                None,
                PeaceLine::Treaty {
                    winner: first_tag(captured.map(|c| c.winners.as_slice()), &deal.winners),
                    loser: first_tag(captured.map(|c| c.losers.as_slice()), &deal.losers),
                    terms,
                    white_peace,
                    members,
                    bounded: deal.end_is_by || (deal.derived && deal.items.iter().any(|i| i.dates.after_hour != i.dates.before_hour)),
                    derived: deal.derived,
                    limitations: deal.limitations.into_iter().filter(|note| Some(note) != deal.capture_unavailable.as_ref()).collect(),
                },
                0,
            );
            let winner_order: BTreeMap<_, _> = deal.winners.iter().enumerate().map(|(i, tag)| (tag, i)).collect();
            let mut recipients: Vec<_> = groups.into_iter().collect();
            recipients.sort_by_key(|(tag, terms)| {
                (tag.as_ref().and_then(|tag| winner_order.get(tag)).copied().unwrap_or(usize::MAX), std::cmp::Reverse(terms.len()))
            });
            for (tag, mut terms) in recipients {
                terms.sort_by_key(|term| term.kind);
                push(Some(group.clone()), PeaceLine::Recipient { tag, terms: counts(terms.iter().map(|t| t.kind)) }, 0);
                for (position, term) in terms.into_iter().enumerate() {
                    push(Some(group.clone()), PeaceLine::Terms { items: vec![term] }, position);
                }
            }
        }
        let mut unread = assembled.unobserved.settlements;
        unread.extend(assembled.unobserved.ownership);
        let gaps = dashboard_world::products::subjects::settle(unread)
            .into_iter()
            .map(|span| MissingSpan {
                first_hour: span.first_hour,
                last_hour: span.last_hour,
                reason: "Peace settlements or ownership outcomes incomplete".into(),
            })
            .collect();
        assembled.warnings.sort_by_key(|w| w.hour);
        Ok(Self { index: Index::new(entries, gaps, &[])?, warnings: assembled.warnings })
    }
}

#[derive(Default)]
struct Branches(HashMap<u64, Arc<Tail>>);

struct Prepared {
    tail: Arc<Tail>,
    selection: Option<Selection>,
    expanded: BTreeSet<String>,
}

fn prepare(source: &Source, query: &Query<'_>, load: &LoadContext, params: &PeaceDealsTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let state = source.state::<Branches>();
    let mut branches = state.lock().unwrap_or_else(PoisonError::into_inner);
    let tail = match branches.0.entry(branch) {
        std::collections::hash_map::Entry::Occupied(entry) => Arc::clone(entry.get()),
        std::collections::hash_map::Entry::Vacant(entry) => {
            Arc::clone(entry.insert(Arc::new(Tail::new(assemble(source, branch, cancelled)?, &query.holders)?)))
        }
    };
    drop(branches);
    let selection = dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    Ok(Prepared { tail, selection, expanded: params.expanded.iter().flatten().cloned().collect() })
}

fn reply(prepared: &Prepared, holders: &Holders, params: &PeaceDealsTailParams, hour: i32) -> io::Result<PeaceDealsTailReply> {
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let index = &prepared.tail.index;
    let pack = params.terms_per_line.unwrap_or(1) as usize;
    let answer = index.ask_visible(
        holders,
        &Ask {
            hour,
            from: params.from,
            to: params.to,
            selection: prepared.selection.as_ref(),
            subject: None,
            kinds: None,
            states: None,
            search: None,
            order: Order::Date(Direction::Descending),
            live_first: false,
            offset,
            limit,
            chips: &[Chip { kinds: Some(vec![0]), states: None }],
        },
        params.to.unwrap_or(i32::MAX),
        |line| line.row.parent.as_ref().is_none_or(|p| prepared.expanded.contains(p)) && line.position.is_multiple_of(pack),
    )?;
    let rows = answer
        .rows
        .iter()
        .map(|shown| {
            let mut row = index.get(shown.at).payload.row.clone();
            if let PeaceLine::Terms { items } = &mut row.detail {
                let start = shown.at as usize + 1;
                for at in start..index.len().min(start + pack - 1) {
                    let next = &index.get(at as u32).payload;
                    let PeaceLine::Terms { items: more } = &next.row.detail else { break };
                    if next.position == 0 {
                        break;
                    }
                    items.extend_from_slice(more);
                }
            }
            row
        })
        .collect();
    let end = prepared.tail.warnings.partition_point(|w| w.hour.is_none_or(|h| h <= hour));
    let mut warnings: Vec<_> = prepared.tail.warnings[..end].iter().map(|w| w.text.clone()).collect();
    if answer.unattributed > 0 {
        warnings.push(format!("The country filter cannot attribute {} peace-history entries to recorded participants.", answer.unattributed));
    }
    let next_change = answer.next_change.into_iter().chain(prepared.tail.warnings.get(end).and_then(|w| w.hour)).min();
    let reply = PeaceDealsTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: offset.to_string(),
        counts: occurrences::counts(&answer, &["treaties"]),
        rows,
        missing: answer.gaps,
        next_change,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct PeaceDealsTail;

impl Product for PeaceDealsTail {
    const KEY: &'static str = "peace-deals-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = PeaceDealsTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, offset, expanded, "expanded[]", termsPerLine, limit
    ];
    type Reply = PeaceDealsTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            PeaceDealsTailParams::decl(),
            PeaceTermKind::decl(),
            PeaceTermCount::decl(),
            PeaceLine::decl(),
            PeaceTerm::decl(),
            PeaceDealsRow::decl(),
            PeaceDealsTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.terms_per_line.is_some_and(|n| !(1..=dashboard_query::series::PAGE_LIMIT).contains(&n)) {
            return Err("Invalid number of terms per line");
        }
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, load, params, cancelled)?;
        reply(&prepared, &query.holders, params, hour)
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid peace-deals-tail panel")?.remove("panel");
    let params: PeaceDealsTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid peace-deals-tail panel: {error}"))?;
    PeaceDealsTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let (first, last, holders) = (query.timeline.first, query.timeline.last, query.holders);
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        check_cancel(cancelled)?;
        if hour < first {
            return Err(dashboard_core::invalid("The hour asked is before the branch"));
        }
        reply(&prepared, &holders, &params, hour.min(last))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::peace::{Joins, PeaceUnobserved, Settled, fold};
    use dashboard_base::span::HourSpan;
    use replay_data::rows::{PeaceSettlementRow, SettledPeaceAction, SettledPeaceSubject, SettledPeaceTransfer};

    #[test]
    fn treaty_and_recipients_appear_at_publication_with_the_unread_stretch() {
        let row = PeaceSettlementRow {
            sequence: 3,
            conference: Some(7),
            hour: Some(40),
            winners: vec![22],
            losers: vec![11],
            actions: vec![SettledPeaceAction { form: 2, taker: 22, country_operand: Some(11), ..Default::default() }],
            subjects: vec![SettledPeaceSubject { overlord: 22, subject: 33 }],
            transfers: vec![SettledPeaceTransfer { state: 9, taker: 22, automatic: true }],
            ..Default::default()
        };
        let joins = Joins { raw: HashMap::from([((22, 40), ((2, 22), "BBB".into())), ((11, 40), ((1, 11), "AAA".into())), ((33, 40), ((3, 33), "CCC".into()))]), ..Default::default() };
        let (deals, warnings) = fold(&[], &[], &[Settled { publication_hour: 43, row }], &joins);
        let holders = Holders::new(BTreeMap::new(), vec![]);
        let unobserved = PeaceUnobserved { settlements: vec![HourSpan { first_hour: 41, last_hour: 42 }], ..Default::default() };
        let tail = Arc::new(Tail::new(Assembled { deals, warnings, unobserved }, &holders).unwrap());
        let expanded = BTreeSet::from(["group:captured:7:3".into()]);
        let prepared = Prepared { tail, selection: None, expanded };
        let mut params: PeaceDealsTailParams = serde_json::from_value(serde_json::json!({"branchId":"1","limit":64})).unwrap();
        let before = reply(&prepared, &holders, &params, 42).unwrap();
        assert!(before.rows.is_empty());
        assert_eq!(before.next_change, Some(43));
        assert_eq!((before.missing[0].first_hour, before.missing[0].last_hour), (41, 42));
        let after = reply(&prepared, &holders, &params, 43).unwrap();
        assert_eq!(after.counts[0].total, "1");
        assert_eq!(after.rows.len(), 4);
        assert!(matches!(&after.rows[0].detail, PeaceLine::Treaty { members: 3, white_peace: false, .. }));
        assert!(matches!(&after.rows[1].detail, PeaceLine::Recipient { tag: Some(tag), .. } if tag == "BBB"));
        assert!(matches!(&after.rows[2].detail, PeaceLine::Terms { items } if items.len() == 1 && items[0].state == Some(9) && items[0].tag.is_none()));
        assert!(matches!(&after.rows[3].detail, PeaceLine::Terms { items } if items.len() == 1 && items[0].tag.as_deref() == Some("CCC")));
        assert_eq!(after.next_change, None);
        params.offset = Some("2".into());
        assert_eq!(reply(&prepared, &holders, &params, 43).unwrap().rows, after.rows[2..]);
    }
}
