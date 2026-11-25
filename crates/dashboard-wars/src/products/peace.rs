
use dashboard_world::products::holder_tags::{HolderTags, Identity};
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::{Query, identity::Holder, log, read};
use replay_data::rows::{PeaceConferenceSpanRow, PeaceOutcomeRow, PeaceSettlementRow, SettledPeaceAction, TableId, tables};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub mod tail;

pub(super) fn deal_hour(deal: &PeaceDeal) -> i32 {
    if let Some(captured) = &deal.captured {
        return captured.hour;
    }
    let base = deal.end_hour.or(deal.first_finalized_hour).unwrap_or(deal.first_observed_hour);
    deal.items.iter().fold(base, |hour, item| hour.max(item.dates.after_hour))
}

pub(super) fn deal_known_at(deal: &PeaceDeal) -> i32 {
    let hour = deal_hour(deal).max(deal.capture_observed_hour.unwrap_or(i32::MIN));
    match (&deal.captured, deal.end_hour) {
        (Some(captured), _) => hour.max(captured.publication_hour),
        (None, Some(_)) => hour.max(deal.first_absent_hour.unwrap_or(hour)),
        (None, None) => hour,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceDates {
    pub before_hour: i32,
    pub after_hour: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeaceGovernment {
    pub ruling_party: Option<u32>,
    pub party_token: Option<String>,
    pub leader_ideology: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeaceChange {
    StateTransfer {
        state: u32,
        from: String,
        to: String,
        from_country: PeaceTransferCountry,
        to_country: PeaceTransferCountry,
        #[doc = " Exact signed sum, null if Provinces cannot supply the state."]
        victory_points: Option<String>,
        victory_points_read_hour: Option<i32>,
    },
    NewSubject {
        overlord: String,
        subject: String,
        autonomy_state: String,
        start_hour: i32,
    },
    GovernmentChange {
        country: String,
        before: PeaceGovernment,
        after: PeaceGovernment,
    },
    CountryReleased {
        country: String,
        #[doc = " Owners of the transferred states at the before read."]
        from: Vec<String>,
        states: Vec<u32>,
        #[doc = " The released country's holder at the after read."]
        to_country: PeaceTransferCountry,
        #[doc = " Each state with its own former owner, in state order."]
        transfers: Vec<PeaceReleasedState>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceReleasedState {
    pub state: u32,
    pub from_country: PeaceTransferCountry,
    pub victory_points: Option<String>,
    pub victory_points_read_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceItem {
    pub derived: bool,
    pub dates: PeaceDates,
    #[doc = " The taker/overlord, null where no enforcer or liberator is recorded."]
    pub winner: Option<String>,
    pub change: PeaceChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeaceObservation {
    RecordedConference,
    ConferenceNotRecorded,
    ConferenceUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceTransferCountry {
    pub slot: u32,
    pub country_id: u32,
    pub tag: String,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceTransferredState {
    pub state: u32,
    pub from_country: PeaceTransferCountry,
    pub to_country: PeaceTransferCountry,
    pub dates: PeaceDates,
    pub victory_points: Option<String>,
    pub victory_points_read_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceCapturedAction {
    pub country: u32,
    pub turn: u32,
    pub ordinal: u32,
    pub giver: u32,
    pub taker_tag: Option<String>,
    pub giver_tag: Option<String>,
    pub form: u8,
    pub state: Option<i32>,
    pub country_operand: Option<u32>,
    pub country_operand_tag: Option<String>,
    pub ship: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceCapturedTransfer {
    pub state: i32,
    pub to: Option<String>,
    pub from: Option<String>,
    pub from_country: Option<PeaceTransferCountry>,
    pub to_country: Option<PeaceTransferCountry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceCapturedSubject {
    pub subject: u32,
    pub overlord_tag: Option<String>,
    pub subject_tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceCaptured {
    pub hour: i32,
    pub publication_hour: i32,
    pub winners: Vec<Option<String>>,
    pub losers: Vec<Option<String>>,
    pub actions: Vec<PeaceCapturedAction>,
    pub transfers: Vec<PeaceCapturedTransfer>,
    pub subjects: Vec<PeaceCapturedSubject>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeaceDeal {
    pub id: String,
    pub derived: bool,
    pub observation: PeaceObservation,
    pub conference_id: Option<String>,
    pub first_observed_hour: i32,
    pub last_observed_hour: i32,
    pub first_finalized_hour: Option<i32>,
    #[doc = " Last seen when followed by a consecutive absence, else first absent hour."]
    pub end_hour: Option<i32>,
    pub end_is_by: bool,
    pub first_absent_hour: Option<i32>,
    pub winners: Vec<String>,
    pub losers: Vec<String>,
    pub items: Vec<PeaceItem>,
    #[doc = " Ordered finalizer operands and outcomes, absent for the derived fallback."]
    pub captured: Option<PeaceCaptured>,
    #[doc = " Reason the finalizer capture cannot certify this deal's actions."]
    pub capture_unavailable: Option<String>,
    #[doc = " The settlement's hour when no conference span supplied this card."]
    pub capture_observed_hour: Option<i32>,
    #[doc = " A failed settlement's hour: its reason is no evidence at an earlier hour."]
    pub capture_unavailable_hour: Option<i32>,
    pub transfers: Vec<PeaceTransferredState>,
    #[doc = " Missing inputs or an attribution limitation, not evidence of no actions."]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PeaceUnobserved {
    pub settlements: Vec<HourSpan>,
    pub ownership: Vec<HourSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub after: i32,
    pub ordinal: u32,
    pub observation: u8,
    pub conference: Option<u64>,
    pub before: i32,
    pub change: u8,
    pub group: u32,
    pub state: u32,
    pub from: Identity,
    pub to: Identity,
    pub victory_points: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Settled {
    pub publication_hour: i32,
    pub row: PeaceSettlementRow,
}

#[derive(Default)]
pub struct Joins {
    pub holders: HolderTags,
    pub raw: HashMap<(u32, i32), (Identity, String)>,
    pub subjects: HashMap<(Identity, Identity, i32), (i32, Option<String>)>,
    pub governments: HashMap<(Identity, i32), PeaceGovernment>,
}

impl Joins {
    fn country(&self, identity: Identity, hour: i32) -> Option<PeaceTransferCountry> {
        let tag = self.holders.tag_at(identity, hour)?;
        Some(PeaceTransferCountry { slot: identity.0, country_id: identity.1, tag: tag.to_owned(), hour })
    }

    fn raw_tag(&self, raw: u32, hour: i32) -> Option<String> {
        self.raw.get(&(raw, hour)).map(|(_, tag)| tag.clone())
    }

    fn raw_country(&self, raw: u32, hour: i32) -> Option<PeaceTransferCountry> {
        self.raw.get(&(raw, hour)).map(|((slot, id), tag)| PeaceTransferCountry { slot: *slot, country_id: *id, tag: tag.clone(), hour })
    }

}

fn card(id: String, observation: PeaceObservation, hour: i32) -> PeaceDeal {
    PeaceDeal {
        id,
        derived: true,
        observation,
        conference_id: None,
        first_observed_hour: hour,
        last_observed_hour: hour,
        first_finalized_hour: None,
        end_hour: None,
        end_is_by: false,
        first_absent_hour: None,
        winners: Vec::new(),
        losers: Vec::new(),
        items: Vec::new(),
        captured: None,
        capture_unavailable: None,
        capture_observed_hour: None,
        capture_unavailable_hour: None,
        transfers: Vec::new(),
        limitations: vec!["Conference participants and war name are not recorded; sides are inferred only from qualifying transfers.".into()],
    }
}

fn vp_text(vp: Option<i64>) -> Option<String> {
    vp.map(|vp| vp.to_string())
}

fn populate(deal: &mut PeaceDeal, rows: &[&Outcome], joins: &Joins) {
    let Some(first) = rows.first() else { return };
    let (before, after) = (first.before, first.after);
    let dates = PeaceDates { before_hour: before, after_hour: after };
    let mut items = Vec::new();
    let mut winners = BTreeSet::new();
    let mut losers = BTreeSet::new();
    let mut unnamed = BTreeSet::new();
    let mut released: BTreeMap<Identity, Vec<&Outcome>> = BTreeMap::new();
    for row in rows {
        match row.change {
            PeaceOutcomeRow::TRANSFER => {
                let (Some(from), Some(to)) = (joins.country(row.from, before), joins.country(row.to, after)) else {
                    unnamed.insert(row.state);
                    continue;
                };
                winners.insert(to.tag.clone());
                losers.insert(from.tag.clone());
                items.push(PeaceItem {
                    derived: true,
                    dates: dates.clone(),
                    winner: Some(to.tag.clone()),
                    change: PeaceChange::StateTransfer {
                        state: row.state,
                        from: from.tag.clone(),
                        to: to.tag.clone(),
                        from_country: from,
                        to_country: to,
                        victory_points: vp_text(row.victory_points),
                        victory_points_read_hour: row.victory_points.map(|_| after),
                    },
                });
            }
            PeaceOutcomeRow::NEW_SUBJECT => {
                let (subject, overlord) = (joins.holders.tag_at(row.from, after), joins.holders.tag_at(row.to, after));
                match (overlord, subject, joins.subjects.get(&(row.to, row.from, after))) {
                    (Some(overlord), Some(subject), Some((start, level))) => items.push(PeaceItem {
                        derived: true,
                        dates: dates.clone(),
                        winner: Some(overlord.to_owned()),
                        change: PeaceChange::NewSubject {
                            overlord: overlord.to_owned(),
                            subject: subject.to_owned(),
                            autonomy_state: level.clone().unwrap_or_default(),
                            start_hour: *start,
                        },
                    }),
                    _ => deal.limitations.push(format!("A new subject at hour {after} is not recorded with its countries and relation; it is not shown.")),
                }
            }
            PeaceOutcomeRow::GOVERNMENT => {
                let Some(country) = joins.holders.tag_at(row.from, after) else {
                    deal.limitations.push(format!("A government change at hour {after} names a country that holds no tag; it is not shown."));
                    continue;
                };
                match (joins.governments.get(&(row.from, before)), joins.governments.get(&(row.from, after))) {
                    (Some(old), Some(new)) => items.push(PeaceItem {
                        derived: true,
                        dates: dates.clone(),
                        winner: None,
                        change: PeaceChange::GovernmentChange { country: country.to_owned(), before: old.clone(), after: new.clone() },
                    }),
                    _ => deal.limitations.push(format!("Government of {country} is not recorded at both reads.")),
                }
            }
            PeaceOutcomeRow::RELEASED => released.entry(row.to).or_default().push(row),
            _ => {}
        }
    }
    for (country, rows) in released {
        let Some(to_country) = joins.country(country, after) else {
            unnamed.extend(rows.iter().map(|row| row.state));
            continue;
        };
        let mut from = BTreeSet::new();
        let mut transfers = Vec::new();
        for row in &rows {
            let Some(owner) = joins.country(row.from, before) else {
                unnamed.insert(row.state);
                continue;
            };
            from.insert(owner.tag.clone());
            transfers.push(PeaceReleasedState {
                state: row.state,
                from_country: owner,
                victory_points: vp_text(row.victory_points),
                victory_points_read_hour: row.victory_points.map(|_| after),
            });
        }
        if transfers.is_empty() {
            continue;
        }
        transfers.sort_by_key(|transfer| transfer.state);
        items.push(PeaceItem {
            derived: true,
            dates: dates.clone(),
            winner: None,
            change: PeaceChange::CountryReleased {
                country: to_country.tag.clone(),
                from: from.into_iter().collect(),
                states: transfers.iter().map(|transfer| transfer.state).collect(),
                to_country,
                transfers,
            },
        });
    }
    if !unnamed.is_empty() {
        deal.limitations.push(format!("States {unnamed:?} changed hands between countries that hold no tag at hours {before} or {after}; the outcome list is incomplete."));
    }
    if items.is_empty() {
        deal.limitations.push("No attributable transfers were observed; an empty list does not prove a white peace or no settlement actions.".into());
    }
    if winners.is_empty() {
        deal.limitations.push("Without qualifying transfers, participant sides and subject/government-only settlements cannot be attributed.".into());
    }
    if before.checked_add(1) != Some(after) {
        deal.limitations.push("The outcome reads span several hours; only their net difference is known, not individual action times.".into());
    }
    deal.winners = winners.into_iter().collect();
    deal.losers = losers.into_iter().collect();
    items.sort_by(|a, b| item_order(a).cmp(&item_order(b)));
    deal.items = items;
}

fn item_order(item: &PeaceItem) -> (u8, &str, u32) {
    match &item.change {
        PeaceChange::StateTransfer { state, to, .. } => (0, to, *state),
        PeaceChange::NewSubject { subject, .. } => (1, subject, 0),
        PeaceChange::GovernmentChange { country, .. } => (2, country, 0),
        PeaceChange::CountryReleased { country, .. } => (3, country, 0),
    }
}

fn conference_card(span: &PeaceConferenceSpanRow, rows: &[&Outcome], joins: &Joins) -> PeaceDeal {
    let mut deal = card(format!("conference:{}:{}", span.conference, span.first_observed_hour), PeaceObservation::RecordedConference, span.first_observed_hour);
    deal.conference_id = Some(span.conference.to_string());
    deal.last_observed_hour = span.last_observed_hour;
    deal.first_finalized_hour = span.first_finalized_hour;
    deal.first_absent_hour = span.first_absent_hour;
    match (span.first_absent_hour, span.first_finalized_hour) {
        (Some(absent), _) => {
            deal.end_is_by = span.last_observed_hour.checked_add(1) != Some(absent);
            deal.end_hour = Some(if deal.end_is_by { absent } else { span.last_observed_hour });
        }
        (None, Some(finalized)) => {
            deal.end_hour = Some(finalized);
            deal.end_is_by = true;
        }
        (None, None) => {}
    }
    match span.comparison {
        PeaceConferenceSpanRow::COMPARED => populate(&mut deal, rows, joins),
        PeaceConferenceSpanRow::NO_BEFORE => deal.limitations.push("No complete ownership and war read before settlement; outcomes are not recorded.".into()),
        PeaceConferenceSpanRow::OVERLAPPED => deal.limitations.push("Sampled conferences overlap this comparison; outcomes cannot be assigned to one.".into()),
        _ if deal.end_hour.is_some() => deal.limitations.push("No complete ownership and war read after settlement; outcomes are not recorded.".into()),
        _ => deal.limitations.push("No end was observed; the conference outcome is not recorded.".into()),
    }
    deal
}

fn outcome_card(rows: &[&Outcome], joins: &Joins) -> PeaceDeal {
    let first = rows[0];
    let observation = if first.observation == PeaceOutcomeRow::NOT_RECORDED { PeaceObservation::ConferenceNotRecorded } else { PeaceObservation::ConferenceUnknown };
    let mut deal = card(format!("outcome:{}:{}", first.after, first.group), observation, first.after);
    deal.end_hour = Some(first.after);
    deal.end_is_by = true;
    populate(&mut deal, rows, joins);
    deal.limitations.push("Connected war-end transfer pairs group this outcome; the data cannot prove how many conferences produced it.".into());
    deal
}

fn captured_action(action: &SettledPeaceAction, joins: &Joins, hour: i32) -> PeaceCapturedAction {
    let tag = |raw: u32| joins.raw_tag(raw, hour);
    PeaceCapturedAction {
        country: action.country,
        turn: action.turn,
        ordinal: action.ordinal,
        giver: action.giver,
        taker_tag: tag(action.taker),
        giver_tag: tag(action.giver),
        form: action.form,
        state: action.state,
        country_operand: action.country_operand,
        country_operand_tag: action.country_operand.and_then(tag),
        ship: action.ship.map(|value| value.to_string()),
    }
}

fn add_transfers(transfers_out: &mut Vec<PeaceTransferredState>, item: &PeaceItem, keep: &dyn Fn(u32) -> bool) {
    match &item.change {
        PeaceChange::StateTransfer { state, from_country, to_country, victory_points, victory_points_read_hour, .. } => transfers_out.push(PeaceTransferredState {
            state: *state,
            from_country: from_country.clone(),
            to_country: to_country.clone(),
            dates: item.dates.clone(),
            victory_points: victory_points.clone(),
            victory_points_read_hour: *victory_points_read_hour,
        }),
        PeaceChange::CountryReleased { to_country, transfers, .. } => {
            for transfer in transfers.iter().filter(|transfer| keep(transfer.state)) {
                transfers_out.push(PeaceTransferredState {
                    state: transfer.state,
                    from_country: transfer.from_country.clone(),
                    to_country: to_country.clone(),
                    dates: item.dates.clone(),
                    victory_points: transfer.victory_points.clone(),
                    victory_points_read_hour: transfer.victory_points_read_hour,
                });
            }
        }
        PeaceChange::NewSubject { .. } | PeaceChange::GovernmentChange { .. } => {}
    }
}

pub fn fold(spans: &[PeaceConferenceSpanRow], outcomes: &[Outcome], settled: &[Settled], joins: &Joins) -> (Vec<PeaceDeal>, Vec<PeaceWarning>) {
    let mut warnings = Vec::new();
    let mut recorded: HashMap<u64, Vec<&Outcome>> = HashMap::new();
    let mut unsampled: BTreeMap<(i32, u32), Vec<&Outcome>> = BTreeMap::new();
    for row in outcomes {
        match (row.observation, row.conference) {
            (PeaceOutcomeRow::RECORDED, Some(conference)) => recorded.entry(conference).or_default().push(row),
            (PeaceOutcomeRow::RECORDED, None) => warnings.push(PeaceWarning::at(row.after, format!("A compared conference's outcome at hour {} names no conference; it is not shown.", row.after))),
            _ => unsampled.entry((row.after, row.group)).or_default().push(row),
        }
    }
    let mut deals: Vec<PeaceDeal> = Vec::new();
    for span in spans {
        let rows: Vec<&Outcome> = recorded.get(&span.conference).into_iter().flatten().copied().filter(|row| Some(row.after) == span.compared_hour).collect();
        deals.push(conference_card(span, &rows, joins));
    }
    for rows in unsampled.values() {
        deals.push(outcome_card(rows, joins));
    }
    let vps: HashMap<(u32, i32), i64> = outcomes.iter().filter_map(|row| Some(((row.state, row.after), row.victory_points?))).collect();
    let mut indices: HashMap<String, Vec<(i32, usize)>> = HashMap::new();
    for (at, deal) in deals.iter().enumerate() {
        if let Some(id) = &deal.conference_id {
            indices.entry(id.clone()).or_default().push((deal.first_observed_hour, at));
        }
    }
    for listings in indices.values_mut() {
        listings.sort_unstable();
    }
    let mut exact_states: HashSet<(u32, i32)> = HashSet::new();
    let mut kept_releases: HashMap<usize, Vec<PeaceItem>> = HashMap::new();
    let mut seen = HashSet::new();
    for Settled { publication_hour, row } in settled {
        if !seen.insert(row.sequence) {
            continue;
        }
        let publication_hour = *publication_hour;
        let Some(id) = row.conference else {
            warnings.push(PeaceWarning::at(publication_hour, format!("A finalizer conference identity was unavailable: {}", String::from_utf8_lossy(row.failure.as_deref().unwrap_or_default()))));
            continue;
        };
        let id = id.to_string();
        let listings = indices.entry(id.clone()).or_default();
        let ends = row.hour.unwrap_or(publication_hour);
        let index = match listings.iter().rev().find(|&&(first, _)| first <= ends).or(listings.first()) {
            Some(&(_, at)) => at,
            None => {
                let mut deal = card(format!("captured:{id}:{}", row.sequence), PeaceObservation::RecordedConference, ends);
                deal.conference_id = Some(id.clone());
                deal.capture_observed_hour = Some(publication_hour);
                deals.push(deal);
                listings.push((ends, deals.len() - 1));
                deals.len() - 1
            }
        };
        let deal = &mut deals[index];
        let Some(hour) = row.hour.filter(|_| row.failure.is_none() && row.omitted == 0) else {
            let reason = format!(
                "Finalizer capture unavailable or truncated (at least {} unread operands): {}",
                row.omitted,
                row.failure.as_deref().map(String::from_utf8_lossy).unwrap_or_else(|| if row.hour.is_none() { "finalizer hour unavailable".into() } else { "capture capacity exceeded".into() })
            );
            deal.capture_unavailable = Some(reason.clone());
            deal.capture_unavailable_hour = Some(publication_hour);
            deal.limitations.push(reason);
            continue;
        };
        deal.derived = false;
        deal.capture_unavailable = None;
        deal.capture_unavailable_hour = None;
        deal.first_finalized_hour = Some(hour);
        deal.last_observed_hour = deal.last_observed_hour.max(hour);
        deal.end_hour = Some(hour);
        deal.end_is_by = false;
        deal.limitations = vec!["Finalization destroys ordered actions; a completed save can validate retained outcomes only.".into()];
        deal.winners = row.winners.iter().filter_map(|&raw| joins.raw_tag(raw, hour)).collect();
        deal.losers = row.losers.iter().filter_map(|&raw| joins.raw_tag(raw, hour)).collect();
        if deal.winners.len() != row.winners.len() || deal.losers.len() != row.losers.len() {
            deal.limitations.push("Some participant tags are not recorded at the finalizer hour; their raw references are preserved.".into());
        }
        let actions: Vec<PeaceCapturedAction> = row.actions.iter().map(|action| captured_action(action, joins, hour)).collect();
        let mut givers: HashMap<i32, Option<u32>> = HashMap::new();
        for action in &actions {
            if let Some(state) = action.state {
                givers.entry(state).and_modify(|old| if *old != Some(action.giver) { *old = None }).or_insert(Some(action.giver));
            }
        }
        let transfers: Vec<PeaceCapturedTransfer> = row
            .transfers
            .iter()
            .map(|transfer| {
                let giver = if transfer.automatic { None } else { givers.get(&transfer.state).copied().flatten() };
                PeaceCapturedTransfer {
                    state: transfer.state,
                    to: joins.raw_tag(transfer.taker, hour),
                    from: giver.and_then(|raw| joins.raw_tag(raw, hour)),
                    from_country: giver.and_then(|raw| joins.raw_country(raw, hour)),
                    to_country: joins.raw_country(transfer.taker, hour),
                }
            })
            .collect();
        let fallback = std::mem::take(&mut deal.items);
        let undrawable: HashSet<u32> = transfers.iter().filter(|t| t.from_country.is_none() || t.to_country.is_none()).filter_map(|t| u32::try_from(t.state).ok()).collect();
        for transfer in &transfers {
            let (Some(from_country), Some(to_country), Ok(state)) = (&transfer.from_country, &transfer.to_country, u32::try_from(transfer.state)) else { continue };
            exact_states.insert((state, publication_hour));
            let vp = vps.get(&(state, publication_hour)).copied();
            deal.items.push(PeaceItem {
                derived: false,
                dates: PeaceDates { before_hour: hour, after_hour: hour },
                winner: Some(to_country.tag.clone()),
                change: PeaceChange::StateTransfer {
                    state,
                    from: from_country.tag.clone(),
                    to: to_country.tag.clone(),
                    from_country: from_country.clone(),
                    to_country: to_country.clone(),
                    victory_points: vp_text(vp),
                    victory_points_read_hour: vp.map(|_| publication_hour),
                },
            });
        }
        let kept: Vec<PeaceItem> = fallback
            .iter()
            .filter(|item| item.derived && item.dates.after_hour == publication_hour)
            .filter_map(|item| {
                let mut kept = item.clone();
                let PeaceChange::CountryReleased { transfers, .. } = &mut kept.change else { return None };
                transfers.retain(|transfer| undrawable.contains(&transfer.state));
                (!transfers.is_empty()).then_some(kept)
            })
            .collect();
        if !kept.is_empty() {
            kept_releases.entry(index).or_default().extend(kept);
        }
        deal.items.extend(fallback.into_iter().filter(|item| {
            item.derived && item.dates.after_hour == publication_hour && matches!(&item.change, PeaceChange::StateTransfer { state, .. } if undrawable.contains(state))
        }));
        if !undrawable.is_empty() {
            deal.limitations.push(format!(
                "{} transfers with unrecorded event-hour endpoint identities remain in captured transfers; the country graph cannot draw those endpoints.",
                transfers.iter().filter(|t| t.from_country.is_none() || t.to_country.is_none()).count()
            ));
        }
        deal.captured = Some(PeaceCaptured {
            hour,
            publication_hour,
            winners: row.winners.iter().map(|&raw| joins.raw_tag(raw, hour)).collect(),
            losers: row.losers.iter().map(|&raw| joins.raw_tag(raw, hour)).collect(),
            actions,
            transfers,
            subjects: row
                .subjects
                .iter()
                .map(|subject| PeaceCapturedSubject {
                    subject: subject.subject,
                    overlord_tag: joins.raw_tag(subject.overlord, hour),
                    subject_tag: joins.raw_tag(subject.subject, hour),
                })
                .collect(),
        });
    }
    for (index, deal) in deals.iter_mut().enumerate() {
        deal.items.retain(|item| !item.derived || !matches!(&item.change, PeaceChange::StateTransfer { state, .. } if exact_states.contains(&(*state, item.dates.after_hour))));
        let kept = kept_releases.remove(&index).unwrap_or_default();
        for item in deal.items.iter().chain(&kept) {
            let keep = |state: u32| !exact_states.contains(&(state, item.dates.after_hour));
            add_transfers(&mut deal.transfers, item, &keep);
        }
    }
    deals.retain(|deal| !deal.derived || deal.conference_id.is_some() || !deal.items.is_empty());
    deals.sort_by(|a, b| b.end_hour.unwrap_or(b.last_observed_hour).cmp(&a.end_hour.unwrap_or(a.last_observed_hour)).then_with(|| a.id.cmp(&b.id)));
    (deals, warnings)
}

fn complement(reads: &[(i32, i32)], first: i32, last: i32, out: &mut Vec<HourSpan>) {
    let mut at = i64::from(first);
    for &(a, b) in reads {
        if i64::from(a) > at {
            out.push(HourSpan { first_hour: at as i32, last_hour: a - 1 });
        }
        at = at.max(i64::from(b) + 1);
    }
    if at <= i64::from(last) {
        out.push(HourSpan { first_hour: at as i32, last_hour: last });
    }
}

fn number(cells: &[log::Cell], at: usize) -> Option<i64> {
    match cells.get(at) {
        Some(log::Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn logged(query: &Query<'_>, name: &str) -> io::Result<Vec<log::Row>> {
    let declared = dashboard_declare::log(name).ok_or_else(|| invalid(format!("No log is declared as {name}")))?;
    let reader = log::Reader::new(query, declared)?;
    let mut rows = Vec::new();
    for stretch in &query.timeline.stretches {
        rows.extend(reader.inserted(stretch.run, &[(stretch.first, stretch.last)])?);
    }
    Ok(rows)
}

fn slot_holders(query: &Query<'_>) -> HashMap<u32, Vec<(u32, String, i32, i32)>> {
    let mut slots: HashMap<u32, Vec<(u32, String, i32, i32)>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                slots.entry(slot).or_default().push((id, tag.clone(), run.first, run.last));
            }
        }
    }
    slots
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceWarning {
    pub hour: Option<i32>,
    pub text: String,
}

impl PeaceWarning {
    fn at(hour: i32, text: String) -> Self {
        Self { hour: Some(hour), text }
    }
}

pub(crate) struct Assembled {
    pub(crate) deals: Vec<PeaceDeal>,
    pub(crate) unobserved: PeaceUnobserved,
    pub(crate) warnings: Vec<PeaceWarning>,
}

pub(crate) fn assemble(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Assembled> {
    let query = dashboard_base::queries::query(source, branch)?;
    let recording = query.replay.recording();
    let mut warnings = Vec::new();
    let mut unobserved = PeaceUnobserved::default();
    let mut rows = Vec::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (first, last, run) = (stretch.first, stretch.last, stretch.run);
        complement(&replay_lanes::lanes::complete_reads(recording, run, TableId::PeaceSettlements, first, last)?, first, last, &mut unobserved.settlements);
        complement(&replay_lanes::lanes::complete_reads(recording, run, TableId::PeaceOutcomes, first, last)?, first, last, &mut unobserved.ownership);
    }
    let span_columns = SPAN_COLUMNS.map(|name| dashboard_declare::field(TableId::PeaceConferenceSpans, name).unwrap_or(usize::MAX));
    if span_columns.contains(&usize::MAX) {
        return Err(invalid("peace conference spans does not hold the columns the peace deals read"));
    }
    for row in logged(&query, "peace-conference-spans")? {
        match conference_span(&row, &span_columns) {
            Some(span) => rows.push(span),
            None => warnings.push(PeaceWarning::at(row.hour, format!("A peace conference's span at hour {} is not recorded in full; it is not shown.", row.hour))),
        }
    }
    for gap in &query.timeline.unheld {
        for held in [&mut unobserved.settlements, &mut unobserved.ownership] {
            held.push(HourSpan { first_hour: gap.first, last_hour: gap.last });
        }
    }
    for held in [&mut unobserved.settlements, &mut unobserved.ownership] {
        *held = dashboard_world::products::subjects::settle(std::mem::take(held));
    }
    let columns: [usize; 11] = [
        "observation", "conference", "before_hour", "change", "group", "state", "from_slot", "from_id", "to_slot", "to_id", "victory_points",
    ]
    .map(|name| dashboard_declare::field(TableId::PeaceOutcomes, name).unwrap_or(usize::MAX));
    if columns.contains(&usize::MAX) {
        return Err(invalid("peace outcomes does not hold the columns the peace deals read"));
    }
    let mut outcomes = Vec::new();
    for row in logged(&query, "peace-outcomes")? {
        match outcome(&row, &columns) {
            Some(outcome) => outcomes.push(outcome),
            None => warnings.push(PeaceWarning::at(row.hour, format!("A peace outcome at hour {} is not recorded in full; it is not shown.", row.hour))),
        }
    }
    dashboard_text::metadata::check_cancel(cancelled)?;
    let settlement_columns = SETTLEMENT_COLUMNS.map(|name| dashboard_declare::field(TableId::PeaceSettlements, name).unwrap_or(usize::MAX));
    if settlement_columns.contains(&usize::MAX) {
        return Err(invalid("peace settlements does not hold the columns the peace deals read"));
    }
    let mut settled = Vec::new();
    for row in logged(&query, "peace-settlements")? {
        match settlement(&row, &settlement_columns) {
            Some(settlement) => settled.push(Settled { publication_hour: row.hour, row: settlement }),
            None => warnings.push(PeaceWarning::at(row.hour, format!("A peace settlement at hour {} is not recorded in full; it is not shown.", row.hour))),
        }
    }
    dashboard_text::metadata::check_cancel(cancelled)?;
    let joins = joins(source, &query, &outcomes, &settled, &mut warnings)?;
    let (deals, more) = fold(&listings(rows), &outcomes, &settled, &joins);
    warnings.extend(more);
    Ok(Assembled { deals, unobserved, warnings })
}

fn listings(rows: Vec<PeaceConferenceSpanRow>) -> Vec<PeaceConferenceSpanRow> {
    let mut spans: BTreeMap<(u64, i32), PeaceConferenceSpanRow> = BTreeMap::new();
    for span in rows {
        let listing = (span.conference, span.first_observed_hour);
        if spans.get(&listing).is_none_or(|held| held.hour <= span.hour) {
            spans.insert(listing, span);
        }
    }
    spans.into_values().collect()
}

const SETTLEMENT_COLUMNS: [&str; 9] = ["conference", "hour", "winners", "losers", "actions", "transfers", "failure", "omitted", "subjects"];

fn settlement(row: &log::Row, columns: &[usize; 9]) -> Option<PeaceSettlementRow> {
    let cell = |index: usize| row.cells.get(columns[index]);
    let optional = |index: usize| match cell(index) {
        Some(log::Cell::Number(value)) => Some(Some(*value)),
        Some(log::Cell::Missing) => Some(None),
        _ => None,
    };
    fn list(cell: Option<&log::Cell>) -> Option<Option<&[u8]>> {
        match cell {
            Some(log::Cell::List(bytes)) if !bytes.is_empty() => Some(Some(&bytes[..])),
            Some(log::Cell::List(_) | log::Cell::Missing) => Some(None),
            _ => None,
        }
    }
    let u32s = |index: usize| -> Option<Vec<u32>> {
        let bytes = list(cell(index))?.unwrap_or_default();
        (bytes.len() % 4 == 0).then(|| bytes.chunks_exact(4).map(|item| u32::from_le_bytes([item[0], item[1], item[2], item[3]])).collect())
    };
    fn decoded<T: replay_data::value::Value>(bytes: Option<&[u8]>) -> Option<T> {
        T::decode(&mut replay_data::codec::Decoder::new(bytes?)).ok()
    }
    fn listed<T: replay_data::value::Value + Default>(bytes: Option<&[u8]>) -> Option<T> {
        match bytes {
            Some(bytes) => T::decode(&mut replay_data::codec::Decoder::new(bytes)).ok(),
            None => Some(T::default()),
        }
    }
    Some(PeaceSettlementRow {
        sequence: row.key as u64,
        conference: optional(0)?.map(|value| value as u64),
        hour: optional(1)?.map(|value| value as i32),
        winners: u32s(2)?,
        losers: u32s(3)?,
        actions: listed(list(cell(4))?)?,
        transfers: listed(list(cell(5))?)?,
        failure: match list(cell(6))? {
            None => None,
            bytes => decoded(bytes)?,
        },
        omitted: number(&row.cells, columns[7])? as u64,
        subjects: listed(list(cell(8))?)?,
    })
}

const SPAN_COLUMNS: [&str; 9] = [
    "first_observed_hour", "last_observed_hour", "current_turn", "finalized", "first_finalized_hour", "first_absent_hour", "comparison", "before_hour", "compared_hour",
];

fn conference_span(row: &log::Row, columns: &[usize; 9]) -> Option<PeaceConferenceSpanRow> {
    let at = |index: usize| number(&row.cells, columns[index]);
    let optional = |index: usize| match row.cells.get(columns[index]) {
        Some(log::Cell::Number(value)) => Some(Some(*value as i32)),
        Some(log::Cell::Missing) => Some(None),
        _ => None,
    };
    Some(PeaceConferenceSpanRow {
        conference: (row.key >> 32) as u64,
        hour: row.key as u32 as i32,
        first_observed_hour: at(0)? as i32,
        last_observed_hour: at(1)? as i32,
        current_turn: at(2)? as i32,
        finalized: at(3)? as u8,
        first_finalized_hour: optional(4)?,
        first_absent_hour: optional(5)?,
        comparison: at(6)? as u8,
        before_hour: optional(7)?,
        compared_hour: optional(8)?,
    })
}

fn outcome(row: &log::Row, columns: &[usize; 11]) -> Option<Outcome> {
    let at = |index: usize| number(&row.cells, columns[index]);
    Some(Outcome {
        after: (row.key >> 32) as u32 as i32,
        ordinal: row.key as u32,
        observation: at(0)? as u8,
        conference: at(1).map(|value| value as u64),
        before: at(2)? as i32,
        change: at(3)? as u8,
        group: at(4)? as u32,
        state: at(5)? as u32,
        from: (at(6)? as u32, at(7)? as u32),
        to: (at(8)? as u32, at(9)? as u32),
        victory_points: at(10),
    })
}

fn joins(source: &Source, query: &Query<'_>, outcomes: &[Outcome], settled: &[Settled], warnings: &mut Vec<PeaceWarning>) -> io::Result<Joins> {
    let recording = query.replay.recording();
    let mut joins = Joins { holders: HolderTags::new(query), ..Joins::default() };
    let slots = slot_holders(query);
    for Settled { publication_hour, row } in settled {
        let hour = row.hour.unwrap_or(*publication_hour);
        let Some(stretch) = query.timeline.at(hour).or_else(|| query.timeline.at(*publication_hour)) else { continue };
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let raws = row
            .winners
            .iter()
            .chain(&row.losers)
            .copied()
            .chain(row.actions.iter().flat_map(|action| [action.country, action.taker, action.giver, action.negotiator]))
            .chain(row.actions.iter().filter_map(|action| action.country_operand))
            .chain(row.transfers.iter().map(|transfer| transfer.taker))
            .chain(row.subjects.iter().flat_map(|subject| [subject.overlord, subject.subject]));
        for raw in raws {
            if joins.raw.contains_key(&(raw, hour)) {
                continue;
            }
            let Some(slot) = links.slot_at(raw, hour) else { continue };
            let holder = slots.get(&slot).and_then(|runs| runs.iter().find(|run| run.2 <= hour && hour <= run.3));
            if let Some((id, tag, ..)) = holder {
                joins.raw.insert((raw, hour), ((slot, *id), tag.clone()));
            }
        }
    }
    for row in outcomes.iter().filter(|row| row.change == PeaceOutcomeRow::NEW_SUBJECT) {
        let Some(stretch) = query.timeline.at(row.after) else { continue };
        let subjects = dashboard_world::products::subjects::cached_subjects(source, stretch.run)?;
        if let Some(relation) = subjects.relation_at(row.to.1, row.from.1, row.after) {
            joins.subjects.insert((row.to, row.from, row.after), relation);
        }
    }
    let mut wanted: BTreeMap<i32, BTreeSet<Identity>> = BTreeMap::new();
    for row in outcomes.iter().filter(|row| row.change == PeaceOutcomeRow::GOVERNMENT) {
        wanted.entry(row.before).or_default().insert(row.from);
        wanted.entry(row.after).or_default().insert(row.from);
    }
    for (hour, countries) in wanted {
        let Some(stretch) = query.timeline.at(hour) else { continue };
        let keys: Vec<u128> = countries.iter().map(|identity| u128::from(identity.1)).collect();
        let reads = [
            read::Read { table: TableId::Politics, columns: None, keys: read::Keys::These(&keys) },
            read::Read::all(TableId::PoliticalParties),
            read::Read { table: TableId::CountryAppearance, columns: None, keys: read::Keys::These(&keys) },
        ];
        let frame = replay_lanes::at::frame(recording, stretch.run, hour, &reads)?;
        let (politics, parties, appearances) = match (frame.complete::<tables::Politics>(), frame.complete::<tables::PoliticalParties>(), frame.complete::<tables::CountryAppearance>()) {
            (Ok(politics), Ok(parties), Ok(appearances)) => (politics, parties, appearances),
            _ => {
                warnings.push(PeaceWarning::at(hour, format!("Governments are not recorded at hour {hour}; government changes there are not shown.")));
                continue;
            }
        };
        for identity in countries {
            let Some(ruling) = politics.iter().find(|row| row.country == identity.1) else { continue };
            let party_token = ruling.ruling_party.and_then(|party| parties.iter().find(|row| row.country == identity.1 && row.id == party)).and_then(|row| std::str::from_utf8(&row.token).ok()).map(str::to_owned);
            let leader_ideology = appearances.iter().find(|row| row.country == identity.1).and_then(|row| std::str::from_utf8(&row.leader_ideology).ok()).map(str::to_owned);
            joins.governments.insert((identity, hour), PeaceGovernment { ruling_party: ruling.ruling_party, party_token, leader_ideology });
        }
    }
    Ok(joins)
}
