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
    prepared::{frame_changes::FrameChanges, prepass::Family},
    products::Product,
};
use dashboard_keyed::daily::DAY_HOURS;
use dashboard_keyed::{
    Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{self, TableId, TableRows, tables},
};
use replay_derive::naval_region_control;
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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
        let Ok(currents) = frame.complete::<tables::NavalSupremacy>() else {
            shared::note(&mut self.refusals, "NavalSupremacy is not recorded");
            return Ok(false);
        };
        if let Some(hour) = frame.tables[Self::READ.index()].sampled_at() {
            values.push("epoch".into(), i128::from(hour));
        }
        let countries = Countries::read(frame);
        // names: TAG@slot|region|raw:<reference>, with raw references distinct;
        // input-current|region|raw is an unresolved-identity operand, not a label.
        // input|table|row|length/fields/mask/word holds reduced diplomacy rows.
        for row in currents {
            let name = shared::country_part(frame, countries.as_ref(), row.country)
                .filter(|n| !n.contains('|'))
                .map_or_else(
                    || format!("input-current|{}|{}", row.region, row.country),
                    |owner| format!("{owner}|{}|raw:{}", row.region, row.country),
                );
            values.push(name, i128::from(row.current));
            values.push(
                format!("region|{}|slot", row.region),
                i128::from(row.region_slot),
            );
        }
        operands::save::<tables::CountryLinks>(frame, values, Clone::clone)?;
        operands::save::<tables::Countries>(frame, values, |r| rows::CountryRow {
            id: r.id,
            canonical_slot: r.canonical_slot,
            tag: r.tag.clone(),
            ..Default::default()
        })?;
        operands::save::<tables::FactionMembers>(frame, values, Clone::clone)?;
        operands::save::<tables::Subjects>(frame, values, |r| rows::SubjectRow {
            overlord: r.overlord,
            subject: r.subject,
            ..Default::default()
        })?;
        operands::save::<tables::WarRelations>(frame, values, |r| rows::WarRelationRow {
            first_country: r.first_country,
            second_country: r.second_country,
            ..Default::default()
        })?;
        operands::save::<tables::AirCountryRelations>(frame, values, Clone::clone)?;
        operands::save::<tables::RelationTuples>(frame, values, |r| rows::RelationTupleRow {
            owner_country: r.owner_country,
            target_country: r.target_country,
            type_id: r.type_id,
            ordinal: r.ordinal,
            first_country: r.first_country,
            second_country: r.second_country,
            ..Default::default()
        })?;
        operands::save::<tables::CountryCounterInputs>(frame, values, |r| {
            rows::CountryCounterInputRow {
                country: r.country,
                forward_visibility: r.forward_visibility.clone(),
                directed_wars: r.directed_wars.clone(),
                ..Default::default()
            }
        })?;
        operands::save::<tables::ReservedCountryNaming>(frame, values, |r| {
            rows::ReservedCountryNamingRow {
                tag: r.tag.clone(),
                faction: r.faction.clone(),
                reverse_visibility: r.reverse_visibility.clone(),
                ..Default::default()
            }
        })?;
        Ok(true)
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
pub(crate) fn restore(
    branch: u64,
    hour: i32,
    values: &BTreeMap<&str, i128>,
    sample: i32,
) -> io::Result<DecodedFrame> {
    let mut frame = operands::blank(branch, hour);
    operands::restore::<tables::CountryLinks>(&mut frame, values)?;
    operands::restore::<tables::Countries>(&mut frame, values)?;
    operands::restore::<tables::FactionMembers>(&mut frame, values)?;
    operands::restore::<tables::Subjects>(&mut frame, values)?;
    operands::restore::<tables::WarRelations>(&mut frame, values)?;
    operands::restore::<tables::AirCountryRelations>(&mut frame, values)?;
    operands::restore::<tables::RelationTuples>(&mut frame, values)?;
    operands::restore::<tables::CountryCounterInputs>(&mut frame, values)?;
    operands::restore::<tables::ReservedCountryNaming>(&mut frame, values)?;
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
    frame.tables[TableId::NavalSupremacy.index()] = replay_frame::frame::SharedView::from(
        TableView::Complete(TableRows::NavalSupremacy(currents)),
    )
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
        let own = rows
            .iter()
            .find(|r| {
                (if r.country == 0 {
                    Some(0)
                } else {
                    countries.as_ref().and_then(|c| c.slot(r.country))
                }) == Some(viewer)
            })
            .map_or(0, |r| r.current);
        result.insert(format!("{region}|own"), i128::from(own));
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
        let share = minimum
            .and_then(|m| naval_region_control::share_sums(frame, viewer, region, m, &links));
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
    let p = shared::params::<NavalSupremacy>(q)?;
    let (first, last) = q.window()?;
    let labels = shared::labels(q);
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |l| l.warnings.clone());
    let mut notes = Refusals::default();
    let mut ids = BTreeSet::new();
    if let Ok(l) = &labels {
        if let Ok(sea) = &l.sea_regions {
            ids.extend(sea);
        }
    }
    let mut days = dashboard_keyed::daily::Builder::default();
    let mut viewer_slots = BTreeSet::new();
    if let Some(mut opened) = q.open::<NavalSupremacyObserver>()? {
        warnings.extend(shared::warnings(&opened.scan.observer.refusals));
        for name in opened.scan.names() {
            if let Some((region, _)) = current_name(name) {
                ids.insert(region);
            }
        }
        let named: Vec<_> = opened
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
        let mut keys = vec![dashboard_keyed::KNOWN, dashboard_keyed::READ];
        keys.extend(named.iter().map(|(_, k)| *k));
        for day in dashboard_keyed::daily::Days::requested(q, first, last).iter() {
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
            let epoch = values
                .get("epoch")
                .copied()
                .and_then(|v| i32::try_from(v).ok());
            // Equal fresh reads live in READ even when the rows and epoch marker
            // did not change during the pre-pass's quiet hours.
            let read = epoch.map(|epoch| {
                raw[1]
                    .and_then(|v| i32::try_from(v).ok())
                    .map_or(epoch, |read| read.max(epoch))
            });
            let Some(read) = read.filter(|h| {
                *h <= day.hour
                    && h.div_euclid(DAY_HOURS) == day.hour.div_euclid(DAY_HOURS)
            }) else {
                notes.note(
                    "NavalSupremacy is outside its witnessed midnight cache epoch",
                    day.hour.to_string(),
                );
                continue;
            };
            let frame = restore(q.branch, day.hour, &values, read)?;
            viewer_slots.extend(shared::viewer_slots(
                &frame,
                Countries::read(&frame).as_ref(),
                &p.tag,
            ));
            let values = project(&frame, &p.tag, &ids, &labels, &mut notes);
            days.push(day, read, values);
        }
    }
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
