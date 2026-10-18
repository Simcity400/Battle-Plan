//! Viewer-owned allied-plan admission.
//!
//! The allied-plan switch is local UI state. It enables a physical country
//! array walk after the viewing country; same-country/alias identity or the
//! sent-volunteer/theatre-host predicate admits each occurrence. The latter
//! is distinct from the relation's co-belligerent cache.

use std::{
    cell::LazyCell,
    collections::{BTreeMap, BTreeSet},
};

use dashboard_text::scope::OriginalTagLists;
use replay_data::{
    rows::CountryCounterInputRow, rows::TableId, rows::TableRows, rows::TableSchema, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

/// Inputs read only when the viewer enables allied plans. LandTheatres and
/// CountryLinks are already dependencies of the ordinary plan walk.
pub(crate) const TABLES: &[TableId] = &[
    TableId::ReservedCountryNaming,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::SentVolunteers,
    TableId::CountryCounterInputs,
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    /// Canonical slots in native walk order. Repeated allied occurrences are
    /// retained: each rebuild advances the plan's shared overlap counters.
    pub countries: Vec<u32>,
    /// Unknown admission is withheld, while independently known plans remain.
    pub refused: BTreeMap<String, usize>,
}

impl Selection {
    pub(crate) fn read(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        ours: u32,
        show_allied: bool,
    ) -> Self {
        let mut out = Self {
            countries: vec![ours],
            ..Self::default()
        };
        if !show_allied {
            return out;
        }
        let lists = OriginalTagLists::read(frame);
        let roster = match lists.country_array() {
            Ok(roster) => roster,
            Err(reason) => {
                out.refuse(reason);
                return out;
            }
        };
        let wars = LazyCell::new(|| {
            pair_index::<tables::WarRelations>(frame, links, |row| {
                (row.first_country, row.second_country)
            })
        });
        let factions = LazyCell::new(|| faction_roster(frame, links));
        let subjects = LazyCell::new(|| {
            pair_index::<tables::Subjects>(frame, links, |row| (row.overlord, row.subject))
        });
        let zero = LazyCell::new(|| zero_visibility(frame, links, ours, || factions.lookup(ours)));
        let hosts = LazyCell::new(|| volunteer_hosts(frame, links, ours));
        let zero_theatres = LazyCell::new(|| reserved_theatres(frame));
        // After returning the viewing country, set the cursor to 1.
        // The recorded physical array retains aliases, order and slot zero.
        for &raw in roster.iter().skip(1) {
            if raw == 0 {
                if ours != 0 {
                    match &*zero {
                        Ok(true) => match &*zero_theatres {
                            Ok(()) => out.countries.push(0),
                            Err(reason) => out.refuse(reason.clone()),
                        },
                        Ok(false) => {}
                        Err(reason) => out.refuse(reason.clone()),
                    }
                }
                continue;
            }
            let Some(&target) = links.get(&raw) else {
                out.refuse("allied-plan country occurrence missing from CountryLinks");
                continue;
            };
            // Native suppresses exact raw identity or nonzero canonical identity.
            if target == ours {
                continue;
            }
            // Native rejects active war before faction, then reaches subjects
            // only when the nonnull same-faction shortcut did not succeed.
            let shared = (|| {
                if wars.matches(ours, target)? {
                    return Ok(false);
                }
                let faction = factions.same(ours, target);
                if faction == Ok(true) {
                    return Ok(true);
                }
                let subject = subjects.matches(ours, target);
                if subject == Ok(true) {
                    return Ok(true);
                }
                faction?;
                subject
            })();
            if shared == Ok(true) {
                out.countries.push(target);
                continue;
            }
            // Require a signed-positive target, a nonempty raw sent list and a matching
            // volunteer theatre. Even null or stale sent references satisfy the count;
            // no division join is needed.
            let volunteer = if raw as i32 > 0 {
                hosts
                    .as_ref()
                    .map_err(|reason| *reason)
                    .and_then(|value| value.matches(target))
            } else {
                Ok(false)
            };
            if volunteer == Ok(true) {
                out.countries.push(target);
                continue;
            }
            if let Err(reason) = shared {
                out.refuse(reason);
            }
            if let Err(reason) = volunteer {
                out.refuse(reason);
            }
        }
        out
    }

    fn refuse(&mut self, reason: impl Into<String>) {
        *self.refused.entry(reason.into()).or_default() += 1;
    }
}

fn slot(links: &BTreeMap<u32, u32>, raw: u32) -> Option<u32> {
    if raw == 0 {
        Some(0)
    } else {
        links.get(&raw).copied()
    }
}

#[derive(Default)]
struct PairIndex {
    known: BTreeSet<(u32, u32)>,
    refused: Option<String>,
}

impl PairIndex {
    fn matches(&self, a: u32, b: u32) -> Result<bool, String> {
        if self.known.contains(&(a, b)) {
            Ok(true)
        } else {
            self.refused.clone().map_or(Ok(false), Err)
        }
    }
}

fn pair_index<T: TableSchema>(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    endpoints: impl Fn(&T::Row) -> (u32, u32),
) -> PairIndex {
    let (rows, incomplete) = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => (rows.rows::<T>(), false),
        TableView::Truncated { rows, .. } => (rows.rows::<T>(), true),
        _ => (None, true),
    };
    let mut index = PairIndex {
        refused: incomplete.then(|| format!("allied-plan {:?} unavailable", T::ID)),
        ..Default::default()
    };
    let Some(rows) = rows else {
        index.refused = Some(format!("allied-plan {:?} unavailable", T::ID));
        return index;
    };
    for row in rows {
        let (a, b) = endpoints(row);
        match (slot(links, a), slot(links, b)) {
            (Some(a), Some(b)) => index.known.extend([(a, b), (b, a)]),
            _ => {
                index.refused.get_or_insert_with(|| {
                    format!("allied-plan {:?} country missing from CountryLinks", T::ID)
                });
            }
        }
    }
    index
}

#[derive(Default)]
struct Factions {
    known: BTreeMap<u32, Result<u64, &'static str>>,
    refused: Option<&'static str>,
}

impl Factions {
    fn lookup(&self, country: u32) -> Result<Option<u64>, String> {
        if let Some(value) = self.known.get(&country) {
            value.map(Some).map_err(String::from)
        } else {
            self.refused.map_or(Ok(None), |reason| Err(reason.into()))
        }
    }

    fn same(&self, a: u32, b: u32) -> Result<bool, String> {
        let Some(faction) = self.lookup(a)? else {
            return Ok(false);
        };
        Ok(self.lookup(b)? == Some(faction))
    }
}

fn faction_roster(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> Factions {
    let (rows, incomplete) = match &*frame.tables[TableId::FactionMembers.index()] {
        TableView::Complete(TableRows::FactionMembers(rows)) => (Some(rows), false),
        TableView::Truncated {
            rows: TableRows::FactionMembers(rows),
            ..
        } => (Some(rows), true),
        _ => (None, true),
    };
    let mut factions = Factions {
        refused: incomplete.then_some("allied-plan FactionMembers unavailable"),
        ..Default::default()
    };
    let Some(rows) = rows else {
        return factions;
    };
    for row in rows {
        let Some(country) = slot(links, row.country) else {
            factions
                .refused
                .get_or_insert("allied-plan FactionMembers country missing from CountryLinks");
            continue;
        };
        let reason = "allied-plan FactionMembers has inconsistent faction identity";
        let value = if row.faction == 0 {
            Err(reason)
        } else {
            Ok(row.faction)
        };
        factions
            .known
            .entry(country)
            .and_modify(|previous| {
                if *previous != value {
                    *previous = Err(reason);
                }
            })
            .or_insert(value);
    }
    factions
}

fn zero_visibility(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
    viewer_faction: impl FnOnce() -> Result<Option<u64>, String>,
) -> Result<bool, String> {
    let rows = frame
        .complete::<tables::CountryCounterInputs>()
        .map_err(|_| "allied-plan reserved-country CountryCounterInputs unavailable")?;
    let mut owners = rows
        .iter()
        .filter(|row| links.get(&row.country) == Some(&ours));
    let row = owners
        .next()
        .ok_or("allied-plan reserved-country viewer operands missing")?;
    if owners.next().is_some() {
        return Err("allied-plan reserved-country viewer operands duplicated".into());
    }
    let [present, retired] = row
        .zero_war
        .require("CountryCounterInputs.zero_war")?
        .ok_or("CountryCounterInputs.zero_war capture refused")?;
    if present > 1 || (present == 0 && retired != 0) {
        return Err("CountryCounterInputs.zero_war has invalid native presence".into());
    }
    if present != 0 && retired == 0 {
        return Ok(false);
    }
    let faction: Result<bool, String> = (|| {
        let ours = viewer_faction()?;
        let Some(ours) = ours else {
            return Ok(false);
        };
        let [reserved] = frame
            .complete::<tables::ReservedCountryNaming>()
            .map_err(|_| "allied-plan reserved faction table unavailable")?
        else {
            return Err("allied-plan reserved faction singleton missing".into());
        };
        let reserved = reserved
            .faction
            .require("ReservedCountryNaming.faction")?
            .ok_or("ReservedCountryNaming.faction capture refused")?;
        Ok(ours != 0 && reserved == ours)
    })();
    if faction == Ok(true) {
        return Ok(true);
    }
    let subject = zero_subject(row);
    if subject == Ok(true) {
        return Ok(true);
    }
    faction?;
    subject
}

fn zero_subject(row: &CountryCounterInputRow) -> Result<bool, String> {
    let subject = row
        .zero_subject
        .require("CountryCounterInputs.zero_subject")?
        .ok_or("CountryCounterInputs.zero_subject capture refused")?;
    match (subject.relation_type, subject.parties) {
        (None, None) => Ok(false),
        (Some(kind), None) if kind != engine_native::csubject_relation::TYPE_VALUE => Ok(false),
        (Some(kind), Some(parties)) if kind == engine_native::csubject_relation::TYPE_VALUE => {
            // Status0 target is raw zero: both native match predicates skip
            // canonical comparison and test exact zero on their own party.
            if parties.contains(&Some(0)) {
                Ok(true)
            } else if parties.iter().all(Option::is_some) {
                Ok(false)
            } else {
                Err("CountryCounterInputs.zero_subject parties incomplete".into())
            }
        }
        _ => Err("CountryCounterInputs.zero_subject has inconsistent native branches".into()),
    }
}

fn reserved_theatres(frame: &DecodedFrame) -> Result<(), String> {
    let rows = frame
        .complete::<tables::LandTheatres>()
        .map_err(|_| "allied-plan admitted reserved-country theatre roster unavailable")?;
    let [reserved] = frame
        .complete::<tables::ReservedCountryNaming>()
        .map_err(|_| "allied-plan reserved theatre certificate unavailable")?
    else {
        return Err("allied-plan reserved theatre certificate singleton missing".into());
    };
    let count = reserved
        .land_theatre_count
        .require("ReservedCountryNaming.land_theatre_count")?
        .ok_or("ReservedCountryNaming.land_theatre_count capture refused")?;
    let mut ordinals = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for row in rows.iter().filter(|row| row.owner == 0) {
        if row.id == 0 || !ids.insert(row.id) || !ordinals.insert(row.ordinal) {
            return Err("allied-plan reserved-country theatre certificate mismatch".into());
        }
    }
    if ordinals.len() != count as usize
        || ordinals
            .iter()
            .enumerate()
            .any(|(index, &ordinal)| index != ordinal as usize)
    {
        return Err("allied-plan reserved-country theatre certificate mismatch".into());
    }
    Ok(())
}

#[derive(Default)]
struct VolunteerHosts {
    known: BTreeSet<u32>,
    refused: Option<&'static str>,
}

impl VolunteerHosts {
    fn matches(&self, target: u32) -> Result<bool, &'static str> {
        if self.known.contains(&target) {
            Ok(true)
        } else {
            self.refused.map_or(Ok(false), Err)
        }
    }
}

fn volunteer_hosts(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
) -> Result<VolunteerHosts, &'static str> {
    let (sent, incomplete) = match &*frame.tables[TableId::SentVolunteers.index()] {
        TableView::Complete(TableRows::SentVolunteers(rows)) => (rows, false),
        TableView::Truncated {
            rows: TableRows::SentVolunteers(rows),
            ..
        } => (rows, true),
        _ => return Err("allied-plan SentVolunteers count unavailable"),
    };
    let owner = |raw| slot(links, raw);
    if !sent.iter().any(|row| owner(row.country) == Some(ours)) {
        if incomplete {
            return Err("allied-plan SentVolunteers count unavailable");
        }
        if sent.iter().any(|row| owner(row.country).is_none()) {
            return Err("allied-plan SentVolunteers owner missing from CountryLinks");
        }
        return Ok(VolunteerHosts::default());
    }
    let (theatres, incomplete) = match &*frame.tables[TableId::LandTheatres.index()] {
        TableView::Complete(TableRows::LandTheatres(rows)) => (rows, false),
        TableView::Truncated {
            rows: TableRows::LandTheatres(rows),
            ..
        } => (rows, true),
        _ => return Err("allied-plan volunteer theatre roster unavailable"),
    };
    let mut out = VolunteerHosts {
        refused: incomplete.then_some("allied-plan volunteer theatre roster unavailable"),
        ..Default::default()
    };
    for theatre in theatres {
        if theatre.volunteers_theatre == 0 {
            continue;
        }
        match owner(theatre.owner) {
            Some(country) if country != ours => continue,
            None => {
                out.refused
                    .get_or_insert("allied-plan volunteer theatre owner missing from CountryLinks");
                continue;
            }
            _ => {}
        }
        match links.get(&(theatre.volunteers_theatre as u32)) {
            Some(&host) => {
                out.known.insert(host);
            }
            None => {
                out.refused
                    .get_or_insert("allied-plan volunteer theatre host missing from CountryLinks");
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "allied/tests.rs"]
mod tests;
