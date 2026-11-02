//! Each part's whole state at one read of its tables ([`observe`]), as the game's rules give it:
//! its values by holder and field, the reasons it refused, and for the civil wars every holder's
//! existence. The operator ([`super::facts`]) reads a part only at an hour that may change it.

use super::{Field, PART_SLOT, Part};
use crate::products::politics_state::Tags;
use dashboard_base::products::country_people;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A value as the operator hands it on: a number, or a content token the emit numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val {
    Int(i128),
    Token(Box<str>),
}

impl Val {
    pub(crate) fn heap(&self) -> usize {
        match self {
            Self::Int(_) => 0,
            Self::Token(token) => token.len(),
        }
    }
}

/// One read of a part.
#[derive(Debug, Default)]
pub struct Observation {
    pub values: HashMap<(u32, Field), Val>,
    /// Each refusal once per row it refused, in the order met.
    pub refusals: Vec<String>,
    /// Of the civil wars: whether each holder owns a state, where the hour says; none when the
    /// lists were not read.
    pub existence: Option<HashMap<u32, bool>>,
}

impl Observation {
    fn refuse(&mut self, reason: impl Into<String>) {
        self.refusals.push(reason.into());
    }

    fn set(&mut self, slot: u32, field: Field, value: Val) {
        self.values.insert((slot, field), value);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(installed_content_core::parser::c_string(bytes)).into_owned()
}

fn token(bytes: &[u8]) -> Option<Val> {
    let token = text(bytes);
    (!token.is_empty()).then(|| Val::Token(token.into()))
}

/// `part` at `frame`'s hour, its holders named by `tags`; none of it when they cannot be.
pub fn observe(frame: &DecodedFrame, tags: Option<&Tags<'_>>, part: Part) -> Observation {
    let mut observed = Observation::default();
    let Some(tags) = tags else {
        return observed;
    };
    let read = match part {
        Part::Elections => elections(frame, tags, &mut observed),
        Part::Exiles => exiles(frame, tags, &mut observed),
        Part::Balances => balances(frame, tags, &mut observed),
        Part::CivilWars => civil_wars(frame, tags, &mut observed),
    };
    if read {
        observed.set(PART_SLOT, Field::Known(part), Val::Int(1));
    }
    observed
}

/// Whether the part was read; the same holds for each part below.
fn elections(frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observation) -> bool {
    let rows = match frame.complete::<tables::Politics>() {
        Ok(rows) => rows,
        Err(error) => {
            observed.refuse(format!("election schedules unavailable: {error}"));
            return false;
        }
    };
    // The game's daily politics, elections included, skip a country owning no states.
    let owned = match frame.complete::<tables::CountryCounterInputs>() {
        Ok(counters) => {
            let mut owned = HashMap::with_capacity(counters.len());
            for counter in counters {
                owned.entry(counter.country).or_insert(counter.owned_state_count);
            }
            Some(owned)
        }
        Err(error) => {
            observed.refuse(format!("country existence unavailable: {error}"));
            None
        }
    };
    // A schedule whose country can't be named leaves the hour's schedules unknown, not ended.
    let mut read = true;
    for row in rows {
        let Some(slot) = tags.slot(row.country) else {
            observed.refuse("an election schedule's country has no identity");
            read = false;
            continue;
        };
        let exists = owned
            .as_ref()
            .and_then(|owned| owned.get(&row.country))
            .map(|&count| count > 0);
        let held = match (row.has_elections.require("election eligibility").ok().copied(), exists) {
            (Some(false), _) | (Some(true), Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            (Some(true), None) => {
                observed.refuse("election eligibility: country existence not recorded");
                None
            }
            (None, _) => {
                observed.refuse("election eligibility not recorded");
                None
            }
        };
        let interval = row.election_interval.require("election interval").ok();
        for (field, value) in [
            (Field::ElectionsHeld, held.map(i128::from)),
            (Field::ElectionsRefused, held.is_none().then_some(1)),
            (Field::ElectionInterval, interval.map(|interval| i128::from(*interval))),
            (
                Field::NextElection,
                country_people::next_election_date(row).map(i128::from),
            ),
            (
                Field::LastElection,
                (!engine_native::cpolitical_status::needs_election_cache(row.last_election))
                    .then(|| i128::from(row.last_election)),
            ),
        ] {
            if let Some(value) = value {
                observed.set(slot, field, Val::Int(value));
            }
        }
    }
    read
}

fn exiles(frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observation) -> bool {
    let rows = match frame.complete::<tables::CountryStabilityInputs>() {
        Ok(rows) => rows,
        Err(error) => {
            observed.refuse(format!("exile status unavailable: {error}"));
            return false;
        }
    };
    let reads = frame.complete::<tables::SliceReads>().ok();
    let scalar = |value: &replay_data::rows::Recorded<Option<i64>>| {
        value.require("exile operand").ok().copied().flatten()
    };
    let small = |value: &replay_data::rows::Recorded<Option<i32>>| {
        value.require("exile operand").ok().copied().flatten()
    };
    // An exile is listed by its host: one whose host or country can't be named leaves the
    // hour's exiles unknown, never one fewer.
    let mut read = true;
    for row in rows {
        let Some(host) = small(&row.exile_host) else {
            observed.refuse("an exile host is not recorded");
            read = false;
            continue;
        };
        if host <= 0 {
            continue;
        }
        let Some(slot) = tags.slot(row.country) else {
            observed.refuse("a government in exile has no identity");
            read = false;
            continue;
        };
        let host = tags.slot(host as u32);
        if host.is_none() {
            observed.refuse("an exile's host has no identity");
            read = false;
        }
        let sliced = reads.and_then(|reads| {
            replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
        });
        for (field, value) in [
            (Field::ExileHost, host.map(i128::from)),
            (Field::ExileLegitimacy, scalar(&row.exile_legitimacy).map(i128::from)),
            (Field::ExileManpower, small(&row.exile_manpower).map(i128::from)),
            (Field::ExileCap, small(&row.exile_manpower_cap).map(i128::from)),
            (Field::ExileLeaders, small(&row.exile_leaders).map(i128::from)),
            (Field::ExileRead, sliced.map(i128::from)),
        ] {
            if let Some(value) = value {
                observed.set(slot, field, Val::Int(value));
            }
        }
    }
    read
}

/// The raw countries `frame`'s exiles are: the rows whose recorded host is positive.
pub(crate) fn exile_countries(frame: &DecodedFrame) -> Vec<u32> {
    frame
        .complete::<tables::CountryStabilityInputs>()
        .map(|rows| {
            rows.iter()
                .filter(|row| {
                    row.exile_host
                        .require("exile operand")
                        .ok()
                        .copied()
                        .flatten()
                        .is_some_and(|host| host > 0)
                })
                .map(|row| row.country)
                .collect()
        })
        .unwrap_or_default()
}

fn balances(frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observation) -> bool {
    let (records, members) = match (
        frame.complete::<tables::PowerBalances>(),
        frame.complete::<tables::PowerBalanceMembers>(),
    ) {
        (Ok(records), Ok(members)) => (records, members),
        (Err(error), _) | (_, Err(error)) => {
            observed.refuse(format!("balance reads unavailable: {error}"));
            return false;
        }
    };
    let records: BTreeMap<u32, _> = records
        .iter()
        .map(|record| (record.ordinal, record))
        .collect();
    // Members in stored record order: the first record listing a holder under an identity is
    // the one the game reads.
    let mut ordered: Vec<_> = members.iter().collect();
    ordered.sort_by_key(|member| (member.record, member.ordinal));
    let mut seen = BTreeSet::new();
    let mut shown = BTreeMap::<u32, String>::new();
    let mut unresolved = false;
    for member in ordered {
        let Some(slot) = tags.slot(member.country) else {
            unresolved = true;
            continue;
        };
        let Some(record) = records.get(&member.record) else {
            observed.refuse("a balance member names a record that is not stored");
            continue;
        };
        let balance = text(&record.balance);
        shown.entry(slot).or_insert_with(|| balance.clone());
        if !seen.insert((slot, balance.clone())) {
            observed.refuse("two records list one country under the same balance");
            continue;
        }
        observed.set(
            slot,
            Field::BalanceValue(balance.clone()),
            Val::Int(i128::from(record.value_raw)),
        );
        observed.set(
            slot,
            Field::BalanceRecord(balance.clone()),
            Val::Int(i128::from(record.ordinal)),
        );
        if let Some(range) = token(&record.active_range) {
            observed.set(slot, Field::BalanceRange(balance.clone()), range);
        }
        if let Some(side) = token(&record.trending) {
            observed.set(slot, Field::BalanceTrending(balance), side);
        }
    }
    if unresolved {
        observed.refuse("a balance member has no identity, so no shown balance is known");
        return true;
    }
    for (slot, balance) in shown {
        observed.set(slot, Field::BalanceShown, Val::Token(balance.into()));
    }
    true
}

/// Each holder's stored civil-war rivals, in the game's own lists: a list is not assumed to be
/// answered by its rival's. Apart from them, the game's own predicate for every listed holder,
/// its inputs indexed once for them all; a list naming a country that no longer exists does not
/// make a civil war. And whether every holder exists, so a side's annexation is told from one
/// it outlived: the emit follows it for each holder ever listed.
fn civil_wars(frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observation) -> bool {
    let rows = match frame.complete::<tables::CountryCivilWars>() {
        Ok(rows) => rows,
        Err(error) => {
            observed.refuse(format!("civil-war lists unavailable: {error}"));
            return false;
        }
    };
    let mut listed = BTreeSet::new();
    let mut read = true;
    for row in rows {
        let (Some(slot), Some(rival)) = (tags.slot(row.country), tags.slot(row.target)) else {
            // The lists are incomplete: what they name at this hour is unknown, not ended.
            observed.refuse("a civil-war list names a country with no identity");
            read = false;
            continue;
        };
        observed.set(slot, Field::CivilWarRival(rival), Val::Int(1));
        listed.extend([slot, rival]);
    }
    let predicate = dashboard_text::identity::CivilWars::new(frame);
    for &slot in &listed {
        match predicate
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|predicate| predicate.holds(slot))
        {
            Ok(true) => observed.set(slot, Field::CivilWar, Val::Int(1)),
            Ok(false) => {}
            Err(error) => {
                observed.refuse(format!("civil-war status unavailable: {error}"));
                read = false;
            }
        }
    }
    let mut existence = HashMap::new();
    if let Ok(predicate) = &predicate {
        for slot in tags.holders() {
            if let Ok(exists) = predicate.exists(slot) {
                existence.insert(slot, exists);
            }
        }
    }
    observed.existence = Some(existence);
    read
}

/// The hours at which an ongoing war relation of `frame` stops being ongoing, which changes the
/// civil-war predicate with no row changing.
pub(crate) fn relation_ends(frame: &DecodedFrame) -> BTreeSet<i32> {
    use replay_frame::frame::TableView;
    let rows = match &*frame.tables[TableId::WarRelations.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
        _ => return BTreeSet::new(),
    };
    rows.rows::<tables::WarRelations>()
        .map(|rows| {
            rows.iter()
                .filter(|row| row.end_hour > row.start_hour)
                .map(|row| row.end_hour)
                .collect()
        })
        .unwrap_or_default()
}
