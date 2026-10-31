//! Country viewpoint only. Prepared inputs keep raw occurrences, not a world
//! viewer-by-region cube. Reply names are `<regionId>|<measure>`, with measure
//! control, friendly, enemy, share, shareNum, shareDen or own. Control and explanatory
//! share have independent operands; raw current survives either refusal.
use crate::{
    operands,
    shared::{self, Choice, Labels},
};
use dashboard_base::{
    catalogue::Refusals,
    prepared::{
        frame_changes::FrameChanges,
        prepass::{Family, HeldBytes},
    },
    products::Product,
};
use dashboard_keyed::daily::DAY_HOURS;
use dashboard_keyed::{
    Delta, Observer, Values,
    delta::same_rows,
    countries::Countries,
    daily::{Day, Days},
    query::{KeyedWindow, Opened, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{self, TableId, TableRows, tables},
};
use replay_derive::naval_region_control;
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use ts_rs::TS;

pub struct NavalSupremacy;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "NavalSupremacyParams")]
pub struct Params {
    pub branch_id: String,
    pub tag: String,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
    #[serde(default = "daily_default")]
    pub daily: bool,
}
fn daily_default() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "NavalSupremacyReply")]
pub struct Reply {
    pub window: KeyedWindow,
    pub day_hours: i32,
    pub regions: Vec<Choice>,
    pub scale: i32,
    pub warnings: Vec<String>,
}
impl Product for NavalSupremacy {
    const KEY: &'static str = "naval-supremacy";
    const CONTENT_DEPENDENT: bool = true;
    type Params = Params;
    type Reply = Reply;
    fn validate(p: &Params) -> Result<(), &'static str> {
        shared::validate(&p.branch_id, p.first_hour, p.last_hour)?;
        shared::tag(&p.tag)?;
        if !p.daily {
            return Err("Naval supremacy requires daily sampling");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        let mut t = vec![Params::decl(), Reply::decl()];
        t.extend(shared::types());
        t
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct NavalSupremacyObserver {
    pub refusals: BTreeMap<String, u64>,
    /// The last known observation, which the next hour's changes are found against; never published.
    #[serde(skip)]
    last: Option<Box<Last>>,
}

/// The last known observation's inputs.
struct Last {
    /// The currents' view, then each of [`INPUTS`]', where complete.
    views: Vec<Option<SharedView>>,
    countries: Option<Countries>,
    /// Each raw reference's owner part; None for an unresolved-identity operand.
    parts: HashMap<u32, Option<String>>,
}

pub static FAMILY: Family = dashboard_keyed::block::family::<NavalSupremacyObserver>(
    "naval-supremacy",
    "naval-supremacy.extract",
    "naval-supremacy.sequential",
    "naval-supremacy.fallback",
);
impl Observer for NavalSupremacyObserver {
    const NAME: &'static str = "naval-supremacy";
    const TABLES: &'static [TableId] = &[
        TableId::NavalSupremacy,
        TableId::CountryLinks,
        TableId::Countries,
        TableId::FactionMembers,
        TableId::Subjects,
        TableId::WarRelations,
        TableId::AirCountryRelations,
        TableId::RelationTuples,
        TableId::CountryCounterInputs,
        TableId::ReservedCountryNaming,
    ];
    const READ: TableId = TableId::NavalSupremacy;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::named(TableId::NavalSupremacy, &["region_slot", "current"])?.when_read(),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
            TableFields::all(TableId::FactionMembers),
            TableFields::named(TableId::Subjects, &[])?,
            TableFields::all(TableId::WarRelations),
            TableFields::all(TableId::AirCountryRelations),
            TableFields::all(TableId::RelationTuples),
            TableFields::named(
                TableId::CountryCounterInputs,
                &["forward_visibility", "directed_wars"],
            )?,
            TableFields::named(
                TableId::ReservedCountryNaming,
                &["tag", "faction", "reverse_visibility"],
            )?,
        ])
    }
    fn changed(changes: &FrameChanges) -> bool {
        Self::TABLES.iter().any(|t| changes.rows_changed[t.index()])
            || changes.slot_read[Self::READ.index()]
            || changes.table_read[Self::READ.index()]
    }
    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        self.last = None;
        let Ok(currents) = frame.complete::<tables::NavalSupremacy>() else {
            shared::note(&mut self.refusals, "NavalSupremacy is not recorded");
            return Ok(false);
        };
        if let Some(hour) = frame.tables[Self::READ.index()].sampled_at() {
            values.push("epoch".into(), i128::from(hour));
        }
        let countries = Countries::read(frame);
        let mut parts = HashMap::new();
        // names: TAG@slot|region|raw:<reference>, with raw references distinct;
        // input-current|region|raw is an unresolved-identity operand, not a label.
        // input|table|row|length/fields/mask/word holds reduced diplomacy rows.
        for row in currents {
            let part = parts
                .entry(row.country)
                .or_insert_with(|| owner_part(frame, countries.as_ref(), row.country));
            values.push(
                current_row_name(part.as_deref(), row.region, row.country),
                i128::from(row.current),
            );
            values.push(
                format!("region|{}|slot", row.region),
                i128::from(row.region_slot),
            );
        }
        operands::save::<tables::CountryLinks>(frame, values, Clone::clone)?;
        operands::save::<tables::Countries>(frame, values, reduce_country)?;
        operands::save::<tables::FactionMembers>(frame, values, Clone::clone)?;
        operands::save::<tables::Subjects>(frame, values, reduce_subject)?;
        operands::save::<tables::WarRelations>(frame, values, reduce_war)?;
        operands::save::<tables::AirCountryRelations>(frame, values, Clone::clone)?;
        operands::save::<tables::RelationTuples>(frame, values, reduce_tuple)?;
        operands::save::<tables::CountryCounterInputs>(frame, values, reduce_counters)?;
        operands::save::<tables::ReservedCountryNaming>(frame, values, reduce_reserved)?;
        self.last = Some(Box::new(Last {
            views: tracked_views(frame),
            countries,
            parts,
        }));
        Ok(true)
    }
    fn observe_changes(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        _: &dashboard_keyed::delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        let Some(mut last) = self.last.take() else {
            return Ok(None);
        };
        let Ok(currents) = frame.complete::<tables::NavalSupremacy>() else {
            return Ok(None);
        };
        match frame.tables[Self::READ.index()].sampled_at() {
            Some(hour) => delta.set("epoch".into(), i128::from(hour)),
            None => delta.remove("epoch".into()),
        }
        let views = std::mem::take(&mut last.views);
        // The part each raw reference had before the identity tables changed it.
        let mut renamed = HashMap::new();
        let held = |table: TableId| views[tracked(table)].as_ref();
        let view = |table: TableId| &frame.tables[table.index()];
        let links_same = same_rows::<tables::CountryLinks>(
            held(TableId::CountryLinks),
            view(TableId::CountryLinks),
            PartialEq::eq,
        );
        // Saved countries compare these same fields: equal ones save nothing new.
        let countries_same = same_rows::<tables::Countries>(
            held(TableId::Countries),
            view(TableId::Countries),
            |a, b| a.canonical_slot == b.canonical_slot && a.tag == b.tag,
        );
        if !(links_same
            && countries_same
            && same_rows::<tables::ReservedCountryNaming>(
                held(TableId::ReservedCountryNaming),
                view(TableId::ReservedCountryNaming),
                |a, b| a.tag == b.tag,
            ))
        {
            last.countries = Countries::read(frame);
            for (&raw, part) in &mut last.parts {
                let now = owner_part(frame, last.countries.as_ref(), raw);
                if *part != now {
                    renamed.insert(raw, std::mem::replace(part, now));
                }
            }
        }
        let Some(held) = views[0].as_ref() else {
            return Ok(None);
        };
        if !SharedView::ptr_eq(held, &frame.tables[Self::READ.index()]) || !renamed.is_empty() {
            let Some(before) = dashboard_keyed::delta::rows::<tables::NavalSupremacy>(held) else {
                return Ok(None);
            };
            let Last {
                countries, parts, ..
            } = &mut *last;
            let mut slots = BTreeSet::new();
            let mut touched = BTreeSet::new();
            let ordered = dashboard_keyed::delta::changed_rows(
                before,
                currents,
                |old, new| {
                    old.region_slot == new.region_slot
                        && old.current == new.current
                        && !renamed.contains_key(&old.country)
                },
                |old, new| {
                    if let Some(old) = old {
                        let part = renamed.get(&old.country).or_else(|| parts.get(&old.country));
                        delta.remove(current_row_name(
                            part.and_then(Option::as_deref),
                            old.region,
                            old.country,
                        ));
                        touched.insert(old.region);
                    }
                    if let Some(new) = new {
                        let part = parts
                            .entry(new.country)
                            .or_insert_with(|| owner_part(frame, countries.as_ref(), new.country));
                        delta.set(
                            current_row_name(part.as_deref(), new.region, new.country),
                            i128::from(new.current),
                        );
                        // A region's slot is first pushed after its first current.
                        if slots.insert(new.region) {
                            region_slot(currents, new.region, delta);
                        }
                    }
                },
            );
            if ordered.is_none() {
                return Ok(None);
            }
            for &region in touched.difference(&slots) {
                region_slot(currents, region, delta);
            }
        }
        let mut held = views.iter().skip(1).map(Option::as_ref);
        let mut next = || held.next().flatten();
        let saved = [
            match next() {
                _ if links_same => Some(()),
                held => operands::save_changes::<tables::CountryLinks>(
                    held,
                    frame,
                    Clone::clone,
                    PartialEq::eq,
                    delta,
                )?,
            },
            match next() {
                _ if countries_same => Some(()),
                held => operands::save_changes::<tables::Countries>(
                    held,
                    frame,
                    reduce_country,
                    |a, b| a.canonical_slot == b.canonical_slot && a.tag == b.tag,
                    delta,
                )?,
            },
            operands::save_changes::<tables::FactionMembers>(
                next(),
                frame,
                Clone::clone,
                PartialEq::eq,
                delta,
            )?,
            // Reduced to its key: rows at equal keys save equal values.
            operands::save_changes::<tables::Subjects>(
                next(),
                frame,
                reduce_subject,
                |_, _| true,
                delta,
            )?,
            operands::save_changes::<tables::WarRelations>(
                next(),
                frame,
                reduce_war,
                |_, _| true,
                delta,
            )?,
            operands::save_changes::<tables::AirCountryRelations>(
                next(),
                frame,
                Clone::clone,
                PartialEq::eq,
                delta,
            )?,
            operands::save_changes::<tables::RelationTuples>(
                next(),
                frame,
                reduce_tuple,
                |a, b| a.first_country == b.first_country && a.second_country == b.second_country,
                delta,
            )?,
            operands::save_changes::<tables::CountryCounterInputs>(
                next(),
                frame,
                reduce_counters,
                |a, b| {
                    a.forward_visibility == b.forward_visibility
                        && a.directed_wars == b.directed_wars
                },
                delta,
            )?,
            operands::save_changes::<tables::ReservedCountryNaming>(
                next(),
                frame,
                reduce_reserved,
                |a, b| {
                    a.tag == b.tag
                        && a.faction == b.faction
                        && a.reverse_visibility == b.reverse_visibility
                },
                delta,
            )?,
        ];
        if saved.iter().any(Option::is_none) {
            return Ok(None);
        }
        last.views = tracked_views(frame);
        self.last = Some(last);
        Ok(Some(true))
    }
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.refusals);
        if let Some(last) = &self.last {
            bytes.add(size_of::<Last>());
            bytes.add(last.views.capacity() * size_of::<Option<SharedView>>());
            for view in last.views.iter().flatten() {
                bytes.view(view);
            }
            if let Some(countries) = &last.countries {
                countries.held(bytes);
            }
            bytes.table::<u32, Option<String>>(last.parts.capacity());
            for part in last.parts.values().flatten() {
                bytes.heap(part);
            }
        }
    }
}

/// The currents' view in `frame`, then each of [`INPUTS`]', where complete.
fn tracked_views(frame: &DecodedFrame) -> Vec<Option<SharedView>> {
    std::iter::once(TableId::NavalSupremacy)
        .chain(INPUTS.iter().map(|(table, _)| *table))
        .map(|table| {
            let view = &frame.tables[table.index()];
            matches!(&**view, TableView::Complete(rows) if rows.table() == table)
                .then(|| view.clone())
        })
        .collect()
}

/// Where `table`'s view is in [`Last::views`].
fn tracked(table: TableId) -> usize {
    1 + INPUTS
        .iter()
        .position(|(input, _)| *input == table)
        .expect("tracked naval input")
}

/// The owner part naming raw reference `raw`'s currents; None for an unresolved-identity operand.
fn owner_part(frame: &DecodedFrame, countries: Option<&Countries>, raw: u32) -> Option<String> {
    shared::country_part(frame, countries, raw).filter(|n| !n.contains('|'))
}

fn current_row_name(part: Option<&str>, region: u32, raw: u32) -> String {
    part.map_or_else(
        || format!("input-current|{region}|{raw}"),
        |owner| format!("{owner}|{region}|raw:{raw}"),
    )
}

/// A region's slot: its last current's, as the last value its currents push.
fn region_slot(currents: &[rows::NavalSupremacyRow], region: u32, delta: &mut Delta) {
    let name = format!("region|{region}|slot");
    let first = u128::from(region) << 32;
    let rows = dashboard_keyed::delta::key_range(currents, first..first + (1 << 32));
    match rows.last() {
        Some(row) => delta.set(name, i128::from(row.region_slot)),
        None => delta.remove(name),
    }
}

fn reduce_country(r: &rows::CountryRow) -> rows::CountryRow {
    rows::CountryRow {
        id: r.id,
        canonical_slot: r.canonical_slot,
        tag: r.tag.clone(),
        ..Default::default()
    }
}
fn reduce_subject(r: &rows::SubjectRow) -> rows::SubjectRow {
    rows::SubjectRow {
        overlord: r.overlord,
        subject: r.subject,
        ..Default::default()
    }
}
fn reduce_war(r: &rows::WarRelationRow) -> rows::WarRelationRow {
    rows::WarRelationRow {
        first_country: r.first_country,
        second_country: r.second_country,
        ..Default::default()
    }
}
fn reduce_tuple(r: &rows::RelationTupleRow) -> rows::RelationTupleRow {
    rows::RelationTupleRow {
        owner_country: r.owner_country,
        target_country: r.target_country,
        type_id: r.type_id,
        ordinal: r.ordinal,
        first_country: r.first_country,
        second_country: r.second_country,
        ..Default::default()
    }
}
fn reduce_counters(r: &rows::CountryCounterInputRow) -> rows::CountryCounterInputRow {
    rows::CountryCounterInputRow {
        country: r.country,
        forward_visibility: r.forward_visibility,
        directed_wars: r.directed_wars.clone(),
        ..Default::default()
    }
}
fn reduce_reserved(r: &rows::ReservedCountryNamingRow) -> rows::ReservedCountryNamingRow {
    rows::ReservedCountryNamingRow {
        tag: r.tag.clone(),
        faction: r.faction,
        reverse_visibility: r.reverse_visibility.clone(),
        ..Default::default()
    }
}
fn current_name(name: &str) -> Option<(u32, u32)> {
    let mut parts = name.split('|');
    let owner = parts.next()?;
    let region = parts.next()?.parse().ok()?;
    let raw = parts.next()?;
    let raw = if owner == "input-current" {
        raw.parse().ok()?
    } else {
        raw.strip_prefix("raw:")?.parse().ok()?
    };
    (parts.next().is_none()).then_some((region, raw))
}
/// What one saved table's operands restore: its view, None where it is not known,
/// and the most words a row of it declares.
type Restored = (Option<SharedView>, usize);

/// Restores one saved table from its values, each row's words bounded by the hour's
/// number of values ([`operands::rows`]).
type Input = fn(&BTreeMap<&str, i128>, usize) -> io::Result<Restored>;

/// The saved tables, in restore order.
const INPUTS: &[(TableId, Input)] = &[
    (TableId::CountryLinks, input::<tables::CountryLinks>),
    (TableId::Countries, input::<tables::Countries>),
    (TableId::FactionMembers, input::<tables::FactionMembers>),
    (TableId::Subjects, input::<tables::Subjects>),
    (TableId::WarRelations, input::<tables::WarRelations>),
    (
        TableId::AirCountryRelations,
        input::<tables::AirCountryRelations>,
    ),
    (TableId::RelationTuples, input::<tables::RelationTuples>),
    (
        TableId::CountryCounterInputs,
        input::<tables::CountryCounterInputs>,
    ),
    (
        TableId::ReservedCountryNaming,
        input::<tables::ReservedCountryNaming>,
    ),
];

fn input<T: rows::TableSchema>(
    values: &BTreeMap<&str, i128>,
    bound: usize,
) -> io::Result<Restored> {
    let rows = operands::rows::<T>(values, bound)?;
    Ok((
        rows.map(|rows| TableView::Complete(T::wrap(rows)).into()),
        operands::most_words::<T>(values),
    ))
}

/// The regions' current rows `values` names, each with its region's runtime slot.
fn currents(values: &BTreeMap<&str, i128>) -> io::Result<Vec<rows::NavalSupremacyRow>> {
    let mut currents = Vec::new();
    for (&name, &current) in values {
        if let Some((region, country)) = current_name(name) {
            let slot = values
                .get(format!("region|{region}|slot").as_str())
                .copied()
                .ok_or_else(|| io::Error::other("Naval region runtime slot is not recorded"))?;
            currents.push(rows::NavalSupremacyRow {
                region,
                country,
                current: i64::try_from(current).map_err(io::Error::other)?,
                region_slot: u32::try_from(slot).map_err(io::Error::other)?,
                ..Default::default()
            });
        }
    }
    currents.sort_by_key(|r| (r.region, r.country));
    Ok(currents)
}

#[cfg(test)]
pub(crate) fn restore(
    branch: u64,
    hour: i32,
    values: &BTreeMap<&str, i128>,
    sample: i32,
) -> io::Result<DecodedFrame> {
    let mut frame = operands::blank(branch, hour);
    for (table, input) in INPUTS {
        if let (Some(view), _) = input(values, values.len())? {
            frame.tables[table.index()] = view;
        }
    }
    frame.tables[TableId::NavalSupremacy.index()] = SharedView::from(TableView::Complete(
        TableRows::NavalSupremacy(currents(values)?),
    ))
    .with_sampled_at(Some(sample));
    Ok(frame)
}

/// Independent native projections, shared by prepared-query parity tests.
pub(crate) fn project(
    frame: &DecodedFrame,
    tag: &str,
    regions: &BTreeSet<u32>,
    labels: &Result<Arc<Labels>, String>,
    notes: &mut Refusals,
) -> BTreeMap<String, i128> {
    // names: REGION|MEASURE, with one independent measure per reply series.
    let mut result = BTreeMap::new();
    let countries = Countries::read(frame);
    let slots = shared::viewer_slots(frame, countries.as_ref(), tag);
    let viewer = match slots.len() {
        1 => *slots.first().unwrap(),
        _ => {
            notes.note(
                "Naval viewer tag is missing or ambiguous; select TAG@slot",
                frame.hour.to_string(),
            );
            return result;
        }
    };
    let links = frame
        .complete::<tables::CountryLinks>()
        .unwrap_or_default()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let mut control_context = naval_region_control::ControlContext::new(frame, viewer, &links);
    for &region in regions {
        let identity = format!("{}:{region}", frame.hour);
        let rows = match naval_region_control::regional_rows(frame, region) {
            Ok(rows) => rows,
            Err(why) => {
                notes.note(why, identity);
                continue;
            }
        };
        let slot = |r: &replay_data::rows::NavalSupremacyRow| {
            if r.country == 0 {
                Some(0)
            } else {
                countries.as_ref().and_then(|c| c.slot(r.country))
            }
        };
        match rows.iter().find(|&&r| slot(r) == Some(viewer)) {
            Some(r) => {
                result.insert(format!("{region}|own"), i128::from(r.current));
            }
            // Zero only once no nonzero current of an unresolved owner could be the viewer's.
            None if rows.iter().any(|&r| r.current != 0 && slot(r).is_none()) => notes.note(
                "Naval current owner identity is unresolved; the viewer's own current is unknown",
                format!("{identity}:own"),
            ),
            None => {
                result.insert(format!("{region}|own"), 0);
            }
        }
        let minimum = labels.as_ref().map_err(Clone::clone).and_then(|l| {
            l.minima.get(&region).cloned().unwrap_or_else(|| {
                Err(format!(
                    "Naval terrain minimum for region {region} is unavailable"
                ))
            })
        });
        match control_context.sums(region) {
            Ok((friendly, enemy)) => {
                result.insert(format!("{region}|friendly"), i128::from(friendly));
                result.insert(format!("{region}|enemy"), i128::from(enemy));
                let control = minimum.clone().and_then(|m| {
                    labels
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|l| l.ratio.clone())
                        .and_then(|ratio| naval_region_control::control(friendly, enemy, m, ratio))
                });
                match control {
                    Ok(c) => {
                        result.insert(
                            format!("{region}|control"),
                            match c {
                                1 => 1,
                                2 => -1,
                                _ => 0,
                            },
                        );
                    }
                    Err(why) => notes.note(why, format!("{identity}:control")),
                }
            }
            Err(why) => notes.note(why, format!("{identity}:control operands")),
        }
        let share = minimum.and_then(|m| control_context.share(region, m));
        match share {
            Ok((num, enemy)) => {
                result.insert(format!("{region}|shareNum"), i128::from(num));
                result.insert(
                    format!("{region}|shareDen"),
                    i128::from(num.wrapping_add(enemy)),
                );
                match naval_region_control::share(num, enemy) {
                    Ok(share) => {
                        result.insert(format!("{region}|share"), i128::from(share));
                    }
                    Err(why) => notes.note(why, format!("{identity}:share")),
                }
            }
            Err(why) => notes.note(why, format!("{identity}:share")),
        }
    }
    result
}
pub fn query(q: &Query<'_>) -> io::Result<serde_json::Value> {
    reply(q, sweep)
}

/// Projects each requested day's restored operands into the reply.
pub(crate) struct Projection<'a> {
    tag: &'a str,
    ids: &'a BTreeSet<u32>,
    labels: &'a Result<Arc<Labels>, String>,
    notes: &'a mut Refusals,
    viewer_slots: BTreeSet<u32>,
    days: dashboard_keyed::daily::Builder,
}

impl Projection<'_> {
    /// The day's supplying read, from its read marker and epoch; None, noted, where
    /// the epoch does not witness the day.
    fn read(&mut self, day: &Day, marker: Option<i128>, epoch: Option<i128>) -> Option<i32> {
        let epoch = epoch.and_then(|v| i32::try_from(v).ok());
        // Equal fresh reads live in READ even when the rows and epoch marker
        // did not change during the pre-pass's quiet hours.
        let read = epoch.map(|epoch| {
            marker
                .and_then(|v| i32::try_from(v).ok())
                .map_or(epoch, |read| read.max(epoch))
        });
        let read = read.filter(|h| {
            *h <= day.hour && h.div_euclid(DAY_HOURS) == day.hour.div_euclid(DAY_HOURS)
        });
        if read.is_none() {
            self.notes.note(
                "NavalSupremacy is outside its witnessed midnight cache epoch",
                day.hour.to_string(),
            );
        }
        read
    }

    fn push(&mut self, day: &Day, read: i32, frame: &DecodedFrame) {
        self.viewer_slots.extend(shared::viewer_slots(
            frame,
            Countries::read(frame).as_ref(),
            self.tag,
        ));
        let values = project(frame, self.tag, self.ids, self.labels, self.notes);
        self.days.push(day, read, values);
    }
}

/// Walks the requested days of a window with the series of the names given.
pub(crate) type Walk = fn(
    &Query<'_>,
    &mut Opened<'_, NavalSupremacyObserver>,
    &[(String, u64)],
    (i32, i32),
    &mut Projection<'_>,
) -> io::Result<()>;

/// Each day's values as one lookup of every series, restored whole.
#[cfg(test)]
pub(crate) fn lookup(
    q: &Query<'_>,
    opened: &mut Opened<'_, NavalSupremacyObserver>,
    named: &[(String, u64)],
    (first, last): (i32, i32),
    out: &mut Projection<'_>,
) -> io::Result<()> {
    let mut keys = vec![dashboard_keyed::KNOWN, dashboard_keyed::READ];
    keys.extend(named.iter().map(|(_, k)| *k));
    for day in Days::requested(q, first, last).iter() {
        dashboard_text::metadata::check_cancel(q.cancelled)?;
        let raw = opened.values_at(&keys, day.hour)?;
        if raw[0] != Some(1) {
            continue;
        }
        let values: BTreeMap<_, _> = named
            .iter()
            .zip(raw.iter().skip(2))
            .filter_map(|((n, _), v)| v.map(|v| (n.as_str(), v)))
            .collect();
        let Some(read) = out.read(day, raw[1], values.get("epoch").copied()) else {
            continue;
        };
        let frame = restore(q.branch, day.hour, &values, read)?;
        out.push(day, read, &frame);
    }
    Ok(())
}

/// The values of the names one restored table reads.
struct Group<'a> {
    values: BTreeMap<&'a str, i128>,
    /// Whether a value changed since `view` was restored.
    changed: bool,
    view: Option<SharedView>,
    /// The most words a row of `view` declares.
    words: usize,
}

/// Each series read once over the requested days and swept day by day, applying only
/// its changes; a table is restored again only on a day one of its values changed.
/// The days project exactly as [`lookup`]'s.
fn sweep(
    q: &Query<'_>,
    opened: &mut Opened<'_, NavalSupremacyObserver>,
    named: &[(String, u64)],
    (first, last): (i32, i32),
    out: &mut Projection<'_>,
) -> io::Result<()> {
    let days = Days::requested(q, first, last);
    let days: Vec<&Day> = days.iter().collect();
    let hours: Vec<i32> = days.iter().map(|day| day.hour).collect();
    let mut keys = vec![dashboard_keyed::KNOWN, dashboard_keyed::READ];
    keys.extend(named.iter().map(|(_, k)| *k));
    let mut changes: Vec<(usize, usize, Option<i128>)> = opened
        .changes_at(&keys, &hours)?
        .into_iter()
        .enumerate()
        .flat_map(|(key, changes)| {
            changes
                .into_iter()
                .map(move |(day, value)| (day, key, value))
        })
        .collect();
    changes.sort_unstable_by_key(|&(day, key, _)| (day, key));
    // Each name once, with the keys naming it (its value is the last of theirs that
    // holds one, as a map collected in key order keeps) and the groups reading it.
    let mut names: Vec<&str> = Vec::new();
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut name_of = Vec::with_capacity(named.len());
    for (at, (name, _)) in named.iter().enumerate() {
        let held = *index.entry(name.as_str()).or_insert_with(|| {
            names.push(name.as_str());
            members.push(Vec::new());
            names.len() - 1
        });
        members[held].push(at);
        name_of.push(held);
    }
    let prefixes: Vec<String> = INPUTS
        .iter()
        .map(|(table, _)| operands::prefix(*table))
        .collect();
    let current_group = INPUTS.len();
    let groups_of: Vec<Vec<usize>> = names
        .iter()
        .map(|name| {
            let mut groups: Vec<usize> = prefixes
                .iter()
                .enumerate()
                .filter(|(_, prefix)| name.starts_with(prefix.as_str()))
                .map(|(group, _)| group)
                .collect();
            if current_name(name).is_some() || name.starts_with("region|") {
                groups.push(current_group);
            }
            groups
        })
        .collect();
    let epoch = index.get("epoch").copied();
    let mut groups: Vec<Group> = (0..=current_group)
        .map(|_| Group {
            values: BTreeMap::new(),
            changed: true,
            view: None,
            words: 0,
        })
        .collect();
    let mut held: Vec<Option<i128>> = vec![None; keys.len()];
    let mut values: Vec<Option<i128>> = vec![None; names.len()];
    // The names holding a value: the size of the hour's whole map.
    let mut present = 0_usize;
    let mut changes = changes.into_iter().peekable();
    for (at, day) in days.iter().enumerate() {
        dashboard_text::metadata::check_cancel(q.cancelled)?;
        while let Some((_, key, value)) = changes.next_if(|change| change.0 == at) {
            held[key] = value;
            let Some(key) = key.checked_sub(2) else {
                continue;
            };
            let name = name_of[key];
            let value = members[name].iter().rev().find_map(|&key| held[key + 2]);
            match (values[name], value) {
                (None, None) => continue,
                (Some(before), Some(now)) if before == now => continue,
                (None, Some(_)) => present += 1,
                (Some(_), None) => present -= 1,
                _ => {}
            }
            values[name] = value;
            for &group in &groups_of[name] {
                let group = &mut groups[group];
                match value {
                    Some(value) => group.values.insert(names[name], value),
                    None => group.values.remove(names[name]),
                };
                group.changed = true;
            }
        }
        if held[0] != Some(1) {
            continue;
        }
        let Some(read) = out.read(day, held[1], epoch.and_then(|name| values[name])) else {
            continue;
        };
        let mut frame = operands::blank(q.branch, day.hour);
        for ((table, input), group) in INPUTS.iter().zip(&mut groups) {
            // Rows that outgrow fewer values refuse as their restore would.
            if group.changed || group.words > present {
                (group.view, group.words) = input(&group.values, present)?;
                group.changed = false;
            }
            if let Some(view) = &group.view {
                frame.tables[table.index()] = view.clone();
            }
        }
        let group = &mut groups[current_group];
        if group.changed {
            group.view = Some(SharedView::from(TableView::Complete(
                TableRows::NavalSupremacy(currents(&group.values)?),
            )));
            group.changed = false;
        }
        if let Some(view) = &group.view {
            frame.tables[TableId::NavalSupremacy.index()] =
                view.clone().with_sampled_at(Some(read));
        }
        out.push(day, read, &frame);
    }
    Ok(())
}

pub(crate) fn reply(q: &Query<'_>, walk: Walk) -> io::Result<serde_json::Value> {
    let p = shared::params::<NavalSupremacy>(q)?;
    let (first, last) = q.window()?;
    let labels = shared::labels(q);
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |l| l.warnings.clone());
    let mut notes = Refusals::default();
    let mut ids = BTreeSet::new();
    let opened = q.open::<NavalSupremacyObserver>()?;
    if let Ok(l) = &labels
        && let Ok(sea) = &l.sea_regions
    {
        ids.extend(sea);
    }
    let mut named = Vec::new();
    if let Some(opened) = &opened {
        warnings.extend(shared::warnings(&opened.scan.observer.refusals));
        for name in opened.scan.names() {
            if let Some((region, _)) = current_name(name) {
                ids.insert(region);
            }
        }
        named = opened
            .scan
            .keyed()
            .filter(|(n, _)| {
                operands::input_name(n)
                    || current_name(n).is_some()
                    || n.starts_with("region|")
                    || *n == "epoch"
            })
            .map(|(n, k)| (n.to_owned(), k))
            .collect();
    }
    let mut out = Projection {
        tag: &p.tag,
        ids: &ids,
        labels: &labels,
        notes: &mut notes,
        viewer_slots: BTreeSet::new(),
        days: dashboard_keyed::daily::Builder::default(),
    };
    if let Some(mut opened) = opened {
        walk(q, &mut opened, &named, (first, last), &mut out)?;
    }
    let Projection {
        viewer_slots, days, ..
    } = out;
    let mut window = days.finish(first, last);
    if viewer_slots.len() > 1 {
        window.series.clear();
        notes.note(
            "Naval viewer tag names different country slots in this window; select TAG@slot",
            p.tag,
        );
    }

    let regions = shared::choices(&ids, &labels, &mut notes);
    warnings.extend(notes.counts("naval supremacy fields"));
    shared::finish(
        q,
        &Reply {
            window,
            day_hours: DAY_HOURS,
            regions,
            scale: shared::SCALE,
            warnings,
        },
    )
}
