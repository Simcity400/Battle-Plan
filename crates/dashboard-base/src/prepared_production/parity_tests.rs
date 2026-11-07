//! The v2 readers answer as the v1 readers did: v1's select and spending
//! algorithms, kept here as the oracle, read the endpoints preparation
//! completed in memory; v2 reads the same branch from its published facts.

use super::read::{Class, Classes, Index, Kind};
use super::*;
use crate::prepared::series::tests::Fixture;
use crate::products::equipment::{Domain, StockVariantName};
use crate::products::equipment_produced::{
    ProducedCount, ProducedGrouping, ProducedKey, ProducedParams, ProducedType,
};
use crate::products::production_spending::{
    ProductionSpendingParams, Purposes, prepared::Params as SpendingParams, reply_prepared,
};
use replay_data::rows::{
    CountryLinkRow, CountryRow, LexicalTokenRow, OutputPurpose, ProductionLineOutputRow,
    ProductionLineRow, RocketProductionOutputRow, TableRows, VariantDefinitionRow,
};
use serde_json::{Value, json};
use std::collections::HashSet;

const DAYS: i32 = 40;

fn day_hour(day: i32) -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::ProductionLineOutput)
    else {
        panic!("ProductionLineOutput is read in a daily slot");
    };
    1000 * replay_data::cadence::STAGGER_PERIOD
        + day * replay_data::cadence::STAGGER_PERIOD
        + slot as i32
}

/// The campaign's countries: tag, native id, slot and the link its lines name.
fn countries(day: i32, extra: u32) -> Vec<(String, u32, u32, u32)> {
    // Germany is another country identity from day 30.
    let germany = if day >= 30 { 9002 } else { 9001 };
    let mut rows = vec![
        ("GER".to_owned(), germany, 7, 11),
        ("ITA".to_owned(), 21, 8, 21),
        ("SOV".to_owned(), 31, 9, 31),
    ];
    for index in 0..extra {
        rows.push((format!("X{index:02}"), 100 + index, 20 + index, 100 + index));
    }
    rows
}

/// One output row of a line, counted since day 0.
fn output(
    line: u64,
    country: u32,
    variant: u64,
    units: i64,
    spent: i64,
    purpose: OutputPurpose,
    exact: bool,
) -> ProductionLineOutputRow {
    ProductionLineOutputRow {
        line,
        country,
        variant,
        units,
        since_hour: day_hour(0),
        observed_since_creation: true,
        exact,
        spent_raw: Recorded::Value(Some(spent)),
        purpose: Recorded::Value(purpose.raw()),
        ..Default::default()
    }
}

/// The frame at `offset` hours into `day`: Germany builds rifles (10), from
/// day 2 a design (60) the variant table only lists from day 3, and from day
/// 5 refits destroyers (30); Italy builds rifles (20, renamed on day 2) and
/// from day 10 a design whose token content cannot type (50), one row of it
/// inexact from day 20; the Soviets build tanks (40) new, converted and with
/// purposes not recorded. Germany also delivers Rocket design 10, whose
/// coverage is lost through day 12 after its read. `extra` more countries
/// build rifles.
fn frame(day: i32, offset: i32, extra: u32) -> DecodedFrame {
    let hour = day_hour(day) + offset;
    let mut frame = crate::test_fixtures::source(1, hour, Some(1));
    let countries = countries(day, extra);
    // Rows in ascending key order (native ids).
    let mut rows: Vec<CountryRow> = countries
        .iter()
        .map(|(tag, id, slot, _)| CountryRow {
            id: *id,
            canonical_slot: *slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect();
    rows.sort_by_key(|row| row.id);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(rows));
    let mut links: Vec<CountryLinkRow> = countries
        .iter()
        .flat_map(|&(_, id, slot, link)| {
            [
                CountryLinkRow {
                    id,
                    canonical_slot: slot,
                },
                CountryLinkRow {
                    id: link,
                    canonical_slot: slot,
                },
            ]
        })
        .collect();
    links.sort_by_key(|link| link.id);
    links.dedup_by_key(|link| link.id);
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    let units = i64::from(day + 1);
    let mut lines = vec![(1, 11, 10), (4, 21, 20), (6, 31, 40)];
    let mut rows = vec![
        output(1, 11, 10, units, 1_000 * units, OutputPurpose::New, true),
        output(4, 21, 20, 2 * units, 700 * units, OutputPurpose::New, true),
        output(6, 31, 40, units, 2_500 * units, OutputPurpose::New, true),
    ];
    if day >= 2 {
        lines.push((3, 11, 60));
        rows.push(output(3, 11, 60, units - 2, 400 * (units - 2), OutputPurpose::New, true));
    }
    if day >= 5 {
        lines.push((2, 11, 30));
        rows.push(output(2, 11, 30, units - 5, -300 * (units - 5), OutputPurpose::Refit, true));
    }
    if day >= 10 {
        lines.push((5, 21, 50));
        rows.push(output(
            5,
            21,
            50,
            units - 10,
            90 * (units - 10),
            OutputPurpose::New,
            day < 20,
        ));
    }
    if day >= 8 {
        let mut converted = output(6, 31, 40, units - 8, 600 * (units - 8), OutputPurpose::Converted, true);
        converted.ordinal = 1;
        rows.push(converted);
    }
    if day >= 15 {
        let mut unknown = output(6, 31, 40, 0, 50 * (units - 15), OutputPurpose::Unknown, true);
        unknown.ordinal = 2;
        rows.push(unknown);
    }
    for index in 0..extra {
        let line = 100 + u64::from(index);
        let link = 100 + index;
        lines.push((line, link, 10));
        rows.push(output(line, link, 10, units, 10 * units, OutputPurpose::New, true));
    }
    lines.sort_unstable_by_key(|&(handle, _, _)| handle);
    rows.sort_unstable_by_key(|row| (row.line, row.ordinal));
    *frame.tables[TableId::ProductionLines.index()] = TableView::Complete(TableRows::ProductionLines(
        lines
            .iter()
            .map(|&(handle, country, variant)| ProductionLineRow {
                handle,
                country,
                variant,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(rows));
    let rocket_lost = day == 12 && offset >= 6;
    *frame.tables[TableId::RocketProductionOutput.index()] = if rocket_lost {
        TableView::Unavailable {
            reason: "synthetic Rocket coverage lost".into(),
        }
    } else {
        let row = |country, variant, units: i64, spent: i64| RocketProductionOutputRow {
            country,
            variant,
            since_hour: day_hour(0),
            exact: true,
            units_exact: true,
            ticks: units.unsigned_abs(),
            deliveries: units.unsigned_abs(),
            units,
            spent_raw: spent,
            ..Default::default()
        };
        TableView::Complete(TableRows::RocketProductionOutput(vec![
            row(0, 0, 0, 0),
            RocketProductionOutputRow {
                ticks: units as u64,
                deliveries: units as u64,
                ..row(11, 0, 0, 0)
            },
            row(11, 10, units, 50 * units),
        ]))
    };
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (1, "infantry_equipment_1"),
            (2, "destroyer_1"),
            (3, "light_tank_1"),
            (4, "mystery_kit"),
        ]
        .into_iter()
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        })
        .collect(),
    ));
    let definition = |id, equipment_token, name: &[u8]| VariantDefinitionRow {
        id,
        equipment_token,
        creator_link: 11,
        display_name: name.to_vec(),
        ..Default::default()
    };
    let mut definitions = vec![
        definition(10, 1, b""),
        definition(20, 1, if day >= 2 { &b"Gewehr 43"[..] } else { &b""[..] }),
        definition(30, 2, b""),
        definition(40, 3, b""),
        definition(50, 4, b""),
    ];
    if day >= 3 {
        definitions.push(definition(60, 3, b""));
    }
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(definitions));
    *frame.tables[TableId::VariantModules.index()] =
        TableView::Complete(TableRows::VariantModules(Vec::new()));
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(Vec::new()));
    frame.metrics.table_ns.fill(0);
    if offset == 0 {
        frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    }
    frame
}

/// The campaign's frames, with one hour of day 25 not recorded.
fn campaign(days: i32, extra: u32) -> Vec<DecodedFrame> {
    (0..days)
        .flat_map(|day| (0..replay_data::cadence::STAGGER_PERIOD).map(move |offset| (day, offset)))
        .filter(|&(day, offset)| (day, offset) != (25, 7))
        .map(|(day, offset)| frame(day, offset, extra))
        .collect()
}

fn format_of(frames: &[DecodedFrame]) -> FormatVersion {
    replay::campaign::Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames[..1].to_vec())],
    )
    .unwrap()
    .format
}

/// What the oracle reads: every endpoint as preparation completed it, the
/// Rocket joins withdrawn since, and the accumulator as of the last flush.
struct Prepared {
    fixture: Fixture,
    pages: Vec<EndpointPage>,
    revoked: Vec<(String, u32, u32, Option<i32>, i32, i32)>,
    state: ProductionAccumulator,
    /// The branch scan, which types and inventories designs on its own.
    scanned: crate::products::equipment::production::ScannedProduced,
}

/// Prepares `frames`, flushing after days 9 and 21 and resuming after the first.
fn prepare(frames: &[DecodedFrame]) -> Prepared {
    let flag = AtomicBool::new(false);
    let format = format_of(frames);
    let mut fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut state = ProductionAccumulator::new(format.version(), 1, frames[0].hour);
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    let (mut pages, mut revoked) = (Vec::new(), Vec::new());
    for frame in frames {
        state
            .ingest_changed(format, frame, &flag, &tracker.observe(frame, format))
            .unwrap();
        let flush = [day_hour(9) + 23, day_hour(21) + 23].contains(&frame.hour);
        if flush {
            write_state(&mut writer, &mut state).unwrap();
            writer.commit().unwrap();
            pages.extend(state.completed.drain(..).map(|(page, _)| page));
            revoked.extend(state.revocations());
            if frame.hour == day_hour(9) + 23 {
                state = latest_state(&fixture.store, &fixture.key, 1).unwrap().unwrap();
                tracker.invalidate();
                writer = fixture.append_writer();
            } else {
                writer = fixture.writer();
            }
        }
    }
    state.flush(&mut writer, &flag).unwrap();
    writer.commit().unwrap();
    pages.extend(state.completed.drain(..).map(|(page, _)| page));
    revoked.extend(state.revocations());
    let replay = replay::campaign::Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames.to_vec())],
    )
    .unwrap();
    let scanned = crate::products::equipment::production::scan(&replay, 1, &flag).unwrap();
    Prepared {
        fixture,
        pages,
        revoked,
        state,
        scanned,
    }
}

/// The content the tests type tokens with: rifles and tanks on land,
/// destroyers at sea; any other token is unresolved.
fn class_of(token: &str, variant: u64) -> Class {
    let resolved = match token {
        "infantry_equipment_1" => Some(("infantry_equipment", 0, Domain::Land)),
        "destroyer_1" => Some(("destroyer", 1, Domain::Naval)),
        "light_tank_1" => Some(("light_tank", 2, Domain::Land)),
        _ => None,
    };
    match resolved {
        Some((key, ordinal, domain)) => Class {
            key: key.to_owned(),
            ordinal: Some(ordinal),
            domain: Some(domain),
            name: Some(format!("{key} name")),
            reason: None,
        },
        None => Class {
            key: if token.is_empty() {
                format!("{}{variant}", equipment::UNRESOLVED_MARK)
            } else {
                format!("{}{token}", equipment::UNRESOLVED_MARK)
            },
            ordinal: None,
            domain: None,
            name: None,
            reason: Some(format!("{token}: synthetic content")),
        },
    }
}

fn classes_of(index: &Index) -> Classes {
    Classes {
        of: index
            .finals
            .iter()
            .enumerate()
            .map(|(variant, kind)| {
                kind.as_ref().map(|kind| {
                    class_of(&index.tokens[kind.token as usize], index.variants[variant])
                })
            })
            .collect(),
        raw_names: index
            .raws
            .iter()
            .map(|raw| equipment::variant_names(raw, None).0)
            .collect(),
        sorted: Classes::sorted(index),
    }
}

/// v1's classification as its readers saw it: the type, names and inventories.
struct OldKind {
    class: Class,
    names: Vec<StockVariantName>,
    seen: [bool; SEEN],
}

impl Prepared {
    fn names(&self, names: &[VariantName]) -> Vec<StockVariantName> {
        names
            .iter()
            .map(|name| match name.raw {
                Some(raw) => {
                    let raw = self.state.raw(raw);
                    StockVariantName {
                        first_hour: name.first_hour,
                        name: equipment::variant_names(raw, None).0,
                        known: true,
                        design: raw.design.clone(),
                    }
                }
                None => StockVariantName {
                    first_hour: name.first_hour,
                    name: None,
                    known: false,
                    design: None,
                },
            })
            .collect()
    }

    /// The catalogue's final classifications.
    /// The catalogue's final classifications, typed and inventoried from the
    /// branch scan: every design an equipment holder produced, and every
    /// Rocket design but the scope marker. Names, which v1 dated by its own
    /// reads, are preparation's.
    fn kinds(&self) -> BTreeMap<u64, OldKind> {
        let mut seen: BTreeMap<u64, [bool; SEEN]> = BTreeMap::new();
        for (rocket, tracks) in [(false, &self.scanned.tags), (true, &self.scanned.rocket.tags)] {
            for track in tracks.values() {
                let units = track.changes.keys().map(|entry| (UNITS_SEEN, entry));
                let ic = track.cost_changes.keys().map(|entry| (IC_SEEN, entry));
                for (measure, &(variant, purpose)) in units.chain(ic) {
                    if !rocket || variant != 0 {
                        seen.entry(variant).or_default()[measure + purpose.index()] = true;
                    }
                }
            }
        }
        seen.into_iter()
            .map(|(variant, seen)| {
                (
                    variant,
                    OldKind {
                        class: class_of(self.scanned.variants.token(variant), variant),
                        names: self
                            .state
                            .classifications
                            .get(&variant)
                            .map(|kind| self.names(&kind.names))
                            .unwrap_or_default(),
                        seen,
                    },
                )
            })
            .collect()
    }

    /// The pages as v1 published them once every withdrawn join was repaired.
    fn repaired(&self) -> Vec<EndpointPage> {
        let mut pages = self.pages.clone();
        for page in &mut pages {
            for row in &mut page.countries {
                let Some(source) = &row.equipment else {
                    continue;
                };
                if self.revoked.iter().any(|(tag, slot, country, observation, read, last)| {
                    *tag == row.tag
                        && (*slot, *country, *observation, *read)
                            == (
                                source.slot,
                                source.country_id,
                                source.observation,
                                source.read_hour,
                            )
                        && page.last_hour >= *read
                        && page.first_hour <= *last
                }) {
                    row.rocket_joined = false;
                }
            }
        }
        pages
    }

    fn diagnostics(&mut self) -> read::Diagnostics {
        read::Diagnostics {
            counts: self.state.final_counts.clone(),
            unnamed: self.state.final_unnamed,
            spending: self.state.spending_facts(),
            variant_refusals: self.state.variant_refusals.clone(),
            observation_starts: self.state.observation_starts(),
        }
    }
}

fn page_at(pages: &[EndpointPage], hour: i32) -> Option<&EndpointPage> {
    let at = pages.partition_point(|page| page.last_hour < hour);
    pages.get(at).filter(|page| page.first_hour <= hour)
}

fn country<'a>(page: Option<&'a EndpointPage>, tag: &str) -> Option<&'a CountryEndpoint> {
    let page = page?;
    page.countries
        .binary_search_by(|country| country.tag.as_str().cmp(tag))
        .ok()
        .map(|index| &page.countries[index])
}

/// v1's `bind_equipment_rockets`: each joined row's Rocket source is the one
/// its supplying equipment read held.
fn bind(pages: &[EndpointPage], page: &mut EndpointPage) {
    for index in 0..page.countries.len() {
        let row = &page.countries[index];
        let rocket = row
            .equipment
            .as_ref()
            .filter(|_| row.rocket_joined)
            .and_then(|equipment| {
                country(page_at(pages, equipment.read_hour), &row.tag)
                    .and_then(|row| row.rocket.clone())
            });
        page.countries[index].rocket = rocket;
    }
}

fn compatible(now: &SourceEndpoint, before: Option<&SourceEndpoint>, measure: ProducedMeasure) -> bool {
    if measure == ProducedMeasure::Total {
        return true;
    }
    let Some(target) = predecessor(now.read_hour, measure) else {
        return false;
    };
    before.is_some_and(|before| {
        before.read_hour == target
            && (now.slot, now.country_id, now.observation, now.segment)
                == (
                    before.slot,
                    before.country_id,
                    before.observation,
                    before.segment,
                )
    })
}

/// v1's `aggregate`.
fn aggregate(
    source: Option<&SourceEndpoint>,
    before: Option<&SourceEndpoint>,
    params: &ProducedParams,
    counted: &dyn Fn(u64, Purpose) -> bool,
) -> Option<i128> {
    let source = source?;
    if !compatible(source, before, params.measure)
        || match params.count {
            ProducedCount::Units => !source.units_known,
            ProducedCount::Ic => !source.spending_known,
        }
    {
        return None;
    }
    let sum = |endpoint: &SourceEndpoint, check_exact: bool| -> Option<i128> {
        let mut sum = 0i128;
        for amount in &endpoint.amounts {
            if !counted(amount.variant, amount.purpose) {
                continue;
            }
            if check_exact
                && match params.count {
                    ProducedCount::Units => !amount.units_exact,
                    ProducedCount::Ic => amount.spending_reason.is_some(),
                }
            {
                return None;
            }
            sum += match params.count {
                ProducedCount::Units => amount.units,
                ProducedCount::Ic => amount.spent,
            };
        }
        Some(sum)
    };
    let current = sum(source, true)?;
    if params.measure == ProducedMeasure::Total {
        return Some(current);
    }
    Some(current - sum(before.expect("compatible predecessor"), false)?)
}

fn combined(left: Option<i128>, right: Option<i128>) -> Option<i128> {
    Some(left? + right?)
}

/// v1's `select_ordered` over one page and its predecessors.
#[allow(clippy::too_many_arguments)]
fn old_select_ordered(
    current: &EndpointPage,
    equipment_before: Option<&EndpointPage>,
    rocket_before: Option<&EndpointPage>,
    classes: &BTreeMap<u64, OldKind>,
    warnings: Vec<String>,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
) -> SelectionPage {
    let included: BTreeSet<&str> = params.included.iter().map(String::as_str).collect();
    let includes_resolved = classes
        .values()
        .any(|kind| kind.class.ordinal.is_some() && included.contains(kind.class.key.as_str()));
    let counted = |variant: u64, purpose: Purpose| {
        (params.refits || purpose != Purpose::Refit)
            && classes.get(&variant).is_some_and(|kind| {
                included.is_empty() || included.contains(kind.class.key.as_str())
            })
    };
    let unresolved = |variant: u64| {
        classes
            .get(&variant)
            .is_none_or(|kind| kind.class.ordinal.is_none())
    };
    let outside = |variant: u64, purpose: Purpose| {
        includes_resolved
            && unresolved(variant)
            && (params.refits || purpose != Purpose::Refit)
            && !counted(variant, purpose)
    };
    type GroupOrder = (usize, String, u64);
    type GroupMembers = (String, bool, BTreeSet<u64>);
    let mut groups: BTreeMap<GroupOrder, GroupMembers> = BTreeMap::new();
    let inventory = match params.count {
        ProducedCount::Units => UNITS_SEEN,
        ProducedCount::Ic => IC_SEEN,
    };
    for (&variant, kind) in classes {
        if !seen_in(seen_mask(&kind.seen), inventory, params.refits) {
            continue;
        }
        if !counted(variant, Purpose::New) {
            continue;
        }
        let class = &kind.class;
        let (order, key, ambiguous) = match params.grouping {
            ProducedGrouping::Type => (
                (class.ordinal.unwrap_or(usize::MAX), class.key.clone(), 0),
                class.key.clone(),
                class.ordinal.is_some(),
            ),
            ProducedGrouping::Variant => (
                (class.ordinal.unwrap_or(usize::MAX), class.key.clone(), variant),
                variant.to_string(),
                false,
            ),
            ProducedGrouping::Domain => {
                let (rank, key) = match class.domain {
                    Some(Domain::Land) => (0, "land"),
                    Some(Domain::Air) => (1, "air"),
                    Some(Domain::Naval) => (2, "naval"),
                    None => (3, equipment::UNRESOLVED_ARM),
                };
                ((rank, String::new(), 0), key.to_owned(), class.domain.is_some())
            }
        };
        groups
            .entry(order)
            .or_insert_with(|| (key, ambiguous, BTreeSet::new()))
            .2
            .insert(variant);
    }
    let mut seen = HashSet::new();
    let selected: Vec<&CountryEndpoint> = if params.top.is_some() {
        current.countries.iter().collect()
    } else {
        params
            .tags
            .iter()
            .filter(|tag| seen.insert(tag.as_str()))
            .filter_map(|tag| country(Some(current), tag))
            .collect()
    };
    let evaluate = |row: &CountryEndpoint, hydrate: bool| -> (Option<i128>, SelectedCountry, bool) {
        let equipment = row.equipment.as_ref().expect("covered equipment endpoint");
        let e_before = country(equipment_before, &row.tag).and_then(|row| row.equipment.as_ref());
        let r_before = country(rocket_before, &row.tag).and_then(|row| row.rocket.as_ref());
        let rocket = row.rocket.as_ref().filter(|rocket| {
            row.rocket_joined
                && (rocket.slot, rocket.country_id) == (equipment.slot, equipment.country_id)
                && match (equipment.observation, rocket.observation) {
                    (Some(left), Some(right)) => left == right,
                    _ => true,
                }
        });
        let e = |filter: &dyn Fn(u64, Purpose) -> bool| {
            aggregate(Some(equipment), e_before, params, filter)
        };
        let r = |filter: &dyn Fn(u64, Purpose) -> bool| aggregate(rocket, r_before, params, filter);
        let scope_known = r(&|variant, _| variant == 0).is_some();
        let rocket_value = |value| if scope_known { value } else { None };
        let e_outside = e(&outside);
        let e_total = e(&counted).filter(|_| e_outside == Some(0));
        let r_outside = r(&outside);
        let r_total = rocket_value(r(&counted).filter(|_| r_outside == Some(0)));
        let total = combined(e_total, r_total);
        let covers = compatible(equipment, e_before, params.measure)
            && match params.count {
                ProducedCount::Units => equipment.units_known,
                ProducedCount::Ic => equipment.spending_known,
            };
        let mut values = Vec::new();
        for (key, ambiguous, variants) in groups.values().filter(|_| hydrate) {
            let group = |variant: u64, purpose: Purpose| {
                (params.refits || purpose != Purpose::Refit) && variants.contains(&variant)
            };
            let unsure = |variant: u64, purpose: Purpose| {
                unresolved(variant)
                    && (params.refits || purpose != Purpose::Refit)
                    && match params.grouping {
                        ProducedGrouping::Type => true,
                        ProducedGrouping::Domain => counted(variant, purpose),
                        ProducedGrouping::Variant => false,
                    }
            };
            let e_unsure = e(&unsure);
            let e_amount = e(&group).filter(|_| !ambiguous || e_unsure == Some(0));
            let r_unsure = r(&unsure);
            let r_amount = rocket_value(r(&group).filter(|_| !ambiguous || r_unsure == Some(0)));
            values.push(SelectedValue {
                key: key.clone(),
                amount: combined(e_amount, r_amount).map(|value| value.to_string()),
                equipment_amount: e_amount.map(|value| value.to_string()),
            });
        }
        (
            total,
            SelectedCountry {
                tag: row.tag.clone(),
                slot: equipment.slot,
                country_id: equipment.country_id,
                segment: equipment.segment.to_string(),
                total: total.map(|value| value.to_string()),
                equipment_total: e_total.map(|value| value.to_string()),
                read_hour: equipment.read_hour,
                observation: equipment.observation,
                values,
            },
            covers,
        )
    };
    let (mut has_known, mut has_unknown) = (false, false);
    let mut rows = Vec::new();
    for row in selected {
        if row.equipment.is_none() {
            continue;
        }
        let (total, country, covers) = evaluate(row, false);
        if covers {
            has_known |= total.is_some();
            has_unknown |= total.is_none();
        }
        rows.push((total, country));
    }
    if ranked || params.top.is_some() {
        rows.retain(|(value, _)| value.is_some_and(|value| params.top.is_none() || value != 0));
        rows.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.tag.cmp(&right.1.tag)));
        if let Some(top) = params.top {
            rows.truncate(top as usize);
        }
    }
    let total_countries = rows.len();
    let type_totals = (params.count == ProducedCount::Units
        && params.measure == ProducedMeasure::Total
        && params.grouping == ProducedGrouping::Type
        && params.included.is_empty())
    .then(|| {
        let mut sums: BTreeMap<String, i128> = BTreeMap::new();
        for (_, row) in &rows {
            let (_, country, _) = evaluate(country(Some(current), &row.tag).unwrap(), true);
            for value in country.values {
                if let Some(amount) = value.amount {
                    *sums.entry(value.key).or_default() += amount.parse::<i128>().unwrap();
                }
            }
        }
        groups
            .values()
            .filter_map(|(key, _, _)| sums.get(key).map(|sum| (key.clone(), sum.to_string())))
            .collect()
    });
    let value_min = rows.iter().filter_map(|row| row.0).min().unwrap_or(0).min(0);
    let value_max = rows.iter().filter_map(|row| row.0).max().unwrap_or(0).max(0);
    let countries = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, row)| evaluate(country(Some(current), &row.tag).unwrap(), true).1)
        .collect();
    let mut types = BTreeMap::new();
    for kind in classes
        .values()
        .filter(|kind| seen_in(seen_mask(&kind.seen), inventory, true))
    {
        types
            .entry((kind.class.ordinal.unwrap_or(usize::MAX), kind.class.key.clone()))
            .or_insert_with(|| ProducedType {
                key: kind.class.key.clone(),
                name: kind.class.name.clone(),
                domain: kind.class.domain,
            });
    }
    SelectionPage {
        type_totals,
        observation: None,
        recorded: false,
        schema: 1,
        branch_id: params.branch_id.clone(),
        first_hour: current.first_hour,
        last_hour: current.last_hour,
        known: has_known && !has_unknown,
        keys: groups
            .values()
            .map(|(key, _, variants)| {
                let kind = &classes[variants.first().unwrap()];
                ProducedKey {
                    key: key.clone(),
                    name: (params.grouping == ProducedGrouping::Type)
                        .then(|| kind.class.name.clone())
                        .flatten(),
                    type_key: (params.grouping == ProducedGrouping::Variant)
                        .then(|| kind.class.key.clone()),
                    domain: if params.grouping == ProducedGrouping::Domain {
                        match key.as_str() {
                            "land" => Some(Domain::Land),
                            "air" => Some(Domain::Air),
                            "naval" => Some(Domain::Naval),
                            _ => None,
                        }
                    } else {
                        kind.class.domain
                    },
                    names: if params.grouping == ProducedGrouping::Variant {
                        kind.names.clone()
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect(),
        types: types.into_values().collect(),
        total_countries,
        offset: offset.to_string(),
        value_min: value_min.to_string(),
        value_max: value_max.to_string(),
        countries,
        warnings,
    }
}

/// v1's `select_at_ordered` over the published pages.
#[allow(clippy::too_many_arguments)]
fn old_select(
    prepared: &mut Prepared,
    pages: &[EndpointPage],
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
) -> Option<SelectionPage> {
    let diagnostics = prepared.diagnostics();
    let warnings = diagnostics.counts.warnings_with(diagnostics.unnamed);
    let observation = {
        let starts = &diagnostics.observation_starts;
        let reached = starts.partition_point(|since| *since <= hour);
        reached.checked_sub(1).map(|index| starts[index])
    };
    let recorded = !diagnostics.spending.reads.is_empty();
    let mut classes = prepared.kinds();
    if params.tags.is_empty() && params.top.is_none() {
        let current = EndpointPage {
            first_hour: hour,
            last_hour: hour,
            equipment_read: None,
            rocket_read: None,
            equipment_origin: None,
            equipment_restart: None,
            rocket_origin: None,
            rocket_restart: None,
            countries: Vec::new(),
            classifications: BTreeMap::new(),
            warnings: Vec::new(),
            unnamed_before: 0,
            unnamed_per_hour: 0,
            unnamed_warning_index: 0,
        };
        let mut selection = old_select_ordered(
            &current, None, None, &classes, warnings, params, offset, limit, ranked,
        );
        selection.keys.clear();
        selection.observation = observation;
        selection.recorded = recorded;
        return Some(selection);
    }
    let mut current = page_at(pages, hour)?.clone();
    bind(pages, &mut current);
    for (variant, kind) in &current.classifications {
        if let Some(class) = classes.get_mut(variant) {
            class.names = prepared.names(&kind.names);
        }
    }
    let before = |read: Option<i32>| {
        read.and_then(|read| predecessor(read, params.measure))
            .and_then(|hour| page_at(pages, hour))
            .cloned()
    };
    let equipment_before = before(current.equipment_read);
    let mut rocket_before = before(current.rocket_read);
    if let Some(page) = &mut rocket_before {
        bind(pages, page);
    }
    let mut selection = old_select_ordered(
        &current,
        equipment_before.as_ref(),
        rocket_before.as_ref(),
        &classes,
        warnings,
        params,
        offset,
        limit,
        ranked,
    );
    selection.observation = observation;
    selection.recorded = recorded;
    Some(selection)
}

/// v1's `read_pages`: one country's spending reply at its dates.
#[allow(clippy::too_many_arguments)]
fn old_spending(
    prepared: &mut Prepared,
    pages: &[EndpointPage],
    bounds: (i32, i32),
    input: &SpendingParams,
    hour: i32,
    end: i32,
    purposes: Purposes,
) -> Value {
    let flag = AtomicBool::new(false);
    let holder = prepared.state.tables.holder_ids.get(&input.tag).copied();
    let mut inventory: BTreeMap<bool, BTreeSet<u64>> = BTreeMap::new();
    let mut selected = Vec::new();
    let mut separate = BTreeSet::new();
    if purposes == Purposes::Split {
        for &(owner, variant, _) in prepared.state.purpose_keys.keys() {
            if Some(owner) == holder {
                separate.insert(variant);
            }
        }
    }
    for ((tag, rocket, refits, variant), id) in prepared.state.spent_keys() {
        if tag != input.tag {
            continue;
        }
        if refits == (rocket || purposes != Purposes::WithoutRefits) {
            inventory.entry(rocket).or_default().insert(variant);
        }
        if refits == (rocket || purposes == Purposes::Combined)
            && (rocket || !separate.contains(&variant))
        {
            selected.push((rocket, variant, id));
        }
    }
    let diagnostics = prepared.diagnostics();
    let mut scanned = diagnostics.spending.scanned(holder, &input.tag);
    let dates: BTreeSet<_> = [
        Some(hour),
        Some(end),
        input.from.and_then(|from| from.checked_sub(1)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let endpoints: BTreeMap<i32, Option<&EndpointPage>> = dates
        .iter()
        .map(|&date| (date, page_at(pages, date)))
        .collect();
    for (&rocket, variants) in &inventory {
        let tags = if rocket {
            &mut scanned.rocket.tags
        } else {
            &mut scanned.tags
        };
        let track = tags.entry(input.tag.clone()).or_default();
        for &variant in variants {
            track.cost_changes.entry((variant, Purpose::New)).or_default();
        }
    }
    let mut kinds: BTreeMap<u64, Kind> = prepared
        .kinds()
        .into_iter()
        .map(|(variant, kind)| {
            (
                variant,
                Kind {
                    key: kind.class.key,
                    ordinal: kind.class.ordinal,
                    domain: kind.class.domain,
                    name: kind.class.name,
                    reason: kind.class.reason,
                    names: Vec::new(),
                },
            )
        })
        .collect();
    let (mut eq_scope, mut ro_scope) = (BTreeMap::new(), BTreeMap::new());
    let unique: BTreeMap<_, _> = endpoints
        .values()
        .flatten()
        .map(|page| ((page.first_hour, page.last_hour), *page))
        .collect();
    for page in unique.values() {
        scanned.observations.extend(
            [page.equipment_origin, page.equipment_restart]
                .into_iter()
                .flatten(),
        );
        scanned.rocket.observations.extend(
            [page.rocket_origin, page.rocket_restart]
                .into_iter()
                .flatten(),
        );
        for (variant, kind) in &page.classifications {
            if let Some(class) = kinds.get_mut(variant) {
                for name in prepared.names(&kind.names) {
                    if class
                        .names
                        .last()
                        .is_none_or(|last| last.first_hour < name.first_hour)
                    {
                        class.names.push(name);
                    }
                }
            }
        }
        for rocket in [false, true] {
            let Some(row) = country(Some(page), &input.tag) else {
                continue;
            };
            let Some(source) = (if rocket { &row.rocket } else { &row.equipment }) else {
                continue;
            };
            if let Some(origin) = source.observation {
                if rocket {
                    scanned.rocket.observations.push(origin);
                } else {
                    scanned.observations.push(origin);
                }
            }
            let (tags, reads, scope) = if rocket {
                (&mut scanned.rocket.tags, &mut scanned.rocket.reads, &mut ro_scope)
            } else {
                (&mut scanned.tags, &mut scanned.reads, &mut eq_scope)
            };
            reads.push(source.read_hour);
            let track = tags.entry(input.tag.clone()).or_default();
            let sample = track.hours.len() as u32;
            track.hours.push(page.first_hour);
            track.last_hours.push(page.last_hour);
            track.starts.push(true);
            track.identities.push((source.slot, source.country_id, 0));
            track.presence.push(source.present);
            scope.insert(page.first_hour, source.campaign_spending_reason);
            let mut entries = BTreeMap::new();
            for amount in &source.amounts {
                entries.insert(
                    (amount.variant, amount.purpose),
                    CostTotal {
                        raw: amount.spent,
                        reason: amount.spending_reason,
                        unproven_raw: amount.unproven,
                    },
                );
            }
            for &variant in inventory.get(&rocket).into_iter().flatten() {
                if !entries.keys().any(|&(id, _)| id == variant) {
                    entries.insert((variant, Purpose::New), CostTotal::default());
                }
            }
            for (entry, value) in entries {
                track.cost_changes.entry(entry).or_default().push((sample, value));
            }
        }
    }
    for reads in [&mut scanned.reads, &mut scanned.rocket.reads] {
        reads.sort_unstable();
        reads.dedup();
    }
    for observations in [&mut scanned.observations, &mut scanned.rocket.observations] {
        observations.sort_unstable();
        observations.dedup();
    }
    let mut reply = reply_prepared(
        &scanned,
        &ProductionSpendingParams {
            branch_id: input.branch_id.clone(),
            tag: input.tag.clone(),
            refits: input.refits,
        },
        (&kinds, &eq_scope, &ro_scope, None),
        refusal_lines(&diagnostics.variant_refusals),
        &flag,
        purposes,
    )
    .unwrap();
    for run in &mut reply.production {
        if run.present == Some(false) {
            run.empty_census_hour = unique
                .values()
                .find(|page| page.first_hour <= run.first_hour && run.first_hour <= page.last_hour)
                .and_then(|page| country(Some(page), &input.tag))
                .and_then(|row| row.equipment.as_ref())
                .map(|source| source.read_hour);
        }
    }
    let source_at = |page: Option<&EndpointPage>, rocket: bool| {
        country(page, &input.tag).and_then(|row| {
            if rocket {
                row.rocket.clone()
            } else {
                row.equipment.clone()
            }
        })
    };
    let mut continuity = BTreeMap::new();
    if let Some(before) = input
        .from
        .and_then(|from| from.checked_sub(1))
        .and_then(|hour| endpoints.get(&hour))
    {
        for date in [hour, end] {
            continuity.insert(
                date,
                [false, true].map(|rocket| {
                    let a = source_at(*before, rocket);
                    let b = source_at(endpoints.get(&date).copied().flatten(), rocket);
                    a.zip(b).is_some_and(|(a, b)| {
                        (a.slot, a.country_id, a.observation, a.segment)
                            == (b.slot, b.country_id, b.observation, b.segment)
                    })
                }),
            );
        }
    }
    let window = (input.from.unwrap_or(bounds.0).max(bounds.0), end.min(bounds.1));
    let peaks = crate::products::production_spending::prepared::peaks_of(
        &prepared.fixture.store,
        &prepared.fixture.key,
        1,
        &selected,
        window,
        &flag,
    )
    .unwrap();
    let mut data = serde_json::to_value(reply).unwrap();
    data["continuity"] = json!(continuity);
    data["peaks"] = json!(peaks);
    data["exactHours"] = json!(dates);
    data
}

fn params(
    tags: &[&str],
    top: Option<u32>,
    measure: ProducedMeasure,
    count: ProducedCount,
    grouping: ProducedGrouping,
    included: &[&str],
    refits: bool,
) -> ProducedParams {
    ProducedParams {
        branch_id: "1".into(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        top,
        measure,
        count,
        grouping,
        included: included.iter().map(|key| (*key).to_owned()).collect(),
        refits,
    }
}

/// Every selection v2 answers equals v1's over the same endpoints: at every
/// endpoint and between, for chosen tags and the top N, ranked or not, each
/// measure, count and grouping, with and without refits, and with included
/// types that leave an unresolved design outside. The campaign has Rocket
/// joins withdrawn after publication, refits, a design read before the
/// variant table names it, an unresolved token, an inexact row, conversions,
/// unrecorded purposes, a recording gap and a change of a country's identity,
/// over two flushes and a resume.
#[test]
fn v2_selections_equal_v1_over_a_synthetic_campaign() {
    let flag = AtomicBool::new(false);
    let mut prepared = prepare(&campaign(DAYS, 0));
    assert!(
        !prepared.revoked.is_empty(),
        "the campaign withdraws a published Rocket join"
    );
    let index = read::read_index(&prepared.fixture.store, &prepared.fixture.key, 1)
        .unwrap()
        .unwrap();
    assert_eq!(index.endpoints.len(), prepared.pages.len());
    let classes = classes_of(&index);
    let pages = prepared.repaired();
    let hours: BTreeSet<i32> = pages
        .iter()
        .step_by(3)
        .flat_map(|page| [page.first_hour, page.last_hour, (page.first_hour + page.last_hour) / 2])
        .chain([day_hour(25) + 7, day_hour(0) - 1, day_hour(DAYS) + 50])
        .collect();
    let selections: Vec<(Vec<&str>, Option<u32>)> = vec![
        (vec!["GER", "ITA", "SOV", "GER", "XYZ"], None),
        (vec!["SOV"], None),
        (Vec::new(), Some(2)),
        (Vec::new(), None),
    ];
    let mut compared = 0;
    for &hour in &hours {
        for (tags, top) in &selections {
            for measure in [
                ProducedMeasure::Total,
                ProducedMeasure::Day,
                ProducedMeasure::Week,
                ProducedMeasure::Month,
            ] {
                for count in [ProducedCount::Units, ProducedCount::Ic] {
                    for grouping in [
                        ProducedGrouping::Type,
                        ProducedGrouping::Variant,
                        ProducedGrouping::Domain,
                    ] {
                        for included in [&[][..], &["infantry_equipment"][..], &["light_tank", "?mystery_kit"][..]] {
                            for refits in [false, true] {
                                let params =
                                    params(tags, *top, measure, count, grouping, included, refits);
                                for ranked in [false, true] {
                                    let expected = old_select(
                                        &mut prepared, &pages, hour, &params, 1, 2, ranked,
                                    );
                                    let actual = select::select_with(
                                        &index,
                                        &classes,
                                        &prepared.fixture.store,
                                        &prepared.fixture.key,
                                        hour,
                                        &params,
                                        1,
                                        2,
                                        ranked,
                                        &flag,
                                    )
                                    .unwrap();
                                    assert_eq!(
                                        actual, expected,
                                        "{hour} {tags:?} {top:?} {measure:?} {count:?} {grouping:?} {included:?} {refits} {ranked}"
                                    );
                                    compared += usize::from(
                                        expected.is_some_and(|page| !page.countries.is_empty()),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(compared > 1000, "the selections hold countries: {compared}");
}

/// Every spending reply v2 answers equals v1's, for each country (one
/// absent), at every endpoint's hour with and without a range start, for
/// each way the reply counts purposes; and the months split by purpose read
/// the same coverage.
#[test]
fn v2_spending_equals_v1_over_a_synthetic_campaign() {
    let flag = AtomicBool::new(false);
    let mut prepared = prepare(&campaign(DAYS, 0));
    let index = read::read_index(&prepared.fixture.store, &prepared.fixture.key, 1)
        .unwrap()
        .unwrap();
    let classes = classes_of(&index);
    let pages = prepared.repaired();
    let bounds = (day_hour(0), day_hour(DAYS) - 1);
    let mut compared = 0;
    for tag in ["GER", "ITA", "SOV", "XYZ"] {
        for page in pages.iter().step_by(2) {
            for from in [None, Some(day_hour(3)), Some(page.first_hour)] {
                for purposes in [Purposes::WithoutRefits, Purposes::Combined, Purposes::Split] {
                    let input = SpendingParams {
                        branch_id: "1".into(),
                        tag: tag.into(),
                        refits: purposes != Purposes::WithoutRefits,
                        from,
                        hour: Some(page.first_hour),
                        end: Some(bounds.1),
                    };
                    let expected = old_spending(
                        &mut prepared,
                        &pages,
                        bounds,
                        &input,
                        page.first_hour,
                        bounds.1,
                        purposes,
                    );
                    let actual = crate::products::production_spending::prepared::read_pages(
                        None,
                        &index,
                        &classes,
                        &prepared.fixture.store,
                        &prepared.fixture.key,
                        bounds,
                        &input,
                        page.first_hour,
                        bounds.1,
                        purposes,
                        &flag,
                    )
                    .unwrap();
                    assert_eq!(actual, expected, "{tag} at {} from {from:?} {purposes:?}", page.first_hour);
                    compared += usize::from(!expected["leaves"].as_array().unwrap().is_empty());
                }
            }
        }
    }
    assert!(compared > 100, "the replies hold leaves: {compared}");
    // Month coverage: each month's last hour reads the endpoint holding it.
    for tag in ["GER", "ITA", "SOV"] {
        let holder = index.tags[tag];
        let runs = index
            .coverage(&prepared.fixture.store, &prepared.fixture.key, holder)
            .unwrap();
        for (_, last) in crate::products::purpose_split::months(bounds.0, bounds.1) {
            let page = page_at(&pages, last);
            let expected = page
                .and_then(|page| country(Some(page), tag))
                .and_then(|row| row.equipment.as_ref())
                .and_then(|source| source.campaign_spending_reason);
            let endpoint = index.endpoint_at(last);
            let actual = endpoint
                .and_then(|endpoint| {
                    let at = runs.partition_point(|(from, _)| *from <= endpoint);
                    at.checked_sub(1).and_then(|at| runs[at].1)
                })
                .flatten();
            assert_eq!(actual, expected, "{tag} through {last}");
            assert_eq!(
                endpoint.map(|endpoint| {
                    let row = index.endpoint(endpoint);
                    (row.equipment_read.is_some(), row.equipment_restart.is_some())
                }),
                page.map(|page| (page.equipment_read.is_some(), page.equipment_restart.is_some()))
            );
        }
    }
}

/// A request's work is bounded by what it reads: spending decodes only its
/// country's leaf groups, whatever other countries hold, and a selection
/// decodes at most one keyframe's blocks per endpoint, however long the
/// campaign.
#[test]
fn reads_decode_only_what_a_request_needs() {
    let flag = AtomicBool::new(false);
    let work = |days: i32, extra: u32, read: &dyn Fn(&Prepared, &Index, &Classes)| {
        let prepared = prepare(&campaign(days, extra));
        let index = read::read_index(&prepared.fixture.store, &prepared.fixture.key, 1)
            .unwrap()
            .unwrap();
        let classes = classes_of(&index);
        read(&prepared, &index, &classes);
        *index.work.lock().unwrap()
    };
    let spending = |prepared: &Prepared, index: &Index, classes: &Classes| {
        let hour = day_hour(22) + 3;
        crate::products::production_spending::prepared::read_pages(
            None,
            index,
            classes,
            &prepared.fixture.store,
            &prepared.fixture.key,
            (day_hour(0), hour),
            &SpendingParams {
                branch_id: "1".into(),
                tag: "SOV".into(),
                refits: true,
                from: Some(day_hour(20)),
                hour: Some(hour),
                end: Some(hour),
            },
            hour,
            hour,
            Purposes::Split,
            &flag,
        )
        .unwrap();
    };
    let few = work(24, 0, &spending);
    let many = work(24, 40, &spending);
    assert!(few.blocks > 0 && few.blocks <= 2, "{few:?}");
    assert_eq!(few, many, "other countries' leaves are not decoded");
    let select = |prepared: &Prepared, index: &Index, classes: &Classes| {
        let hour = day_hour(22) + 3;
        let params = params(
            &[],
            Some(5),
            ProducedMeasure::Week,
            ProducedCount::Units,
            ProducedGrouping::Type,
            &[],
            false,
        );
        select::select_with(
            index,
            classes,
            &prepared.fixture.store,
            &prepared.fixture.key,
            hour,
            &params,
            0,
            5,
            true,
            &flag,
        )
        .unwrap()
        .unwrap();
    };
    let short = work(24, 0, &select);
    let long = work(DAYS, 0, &select);
    // The endpoint, its predecessor and their supplying reads, each from its
    // keyframe at most.
    assert!(
        short.blocks <= 4 * facts::RANKING_KEYFRAME,
        "{short:?}"
    );
    assert_eq!(short, long, "a selection's work does not grow with the campaign");
}

#[test]
fn equipment_produced_validation_still_refuses_both_tags_and_top() {
    let flag = AtomicBool::new(false);
    let prepared = prepare(&campaign(3, 0));
    let index = read::read_index(&prepared.fixture.store, &prepared.fixture.key, 1)
        .unwrap()
        .unwrap();
    let classes = classes_of(&index);
    let both = params(
        &["GER"],
        Some(1),
        ProducedMeasure::Total,
        ProducedCount::Units,
        ProducedGrouping::Type,
        &[],
        false,
    );
    let error = select::select_with(
        &index,
        &classes,
        &prepared.fixture.store,
        &prepared.fixture.key,
        day_hour(1),
        &both,
        0,
        1,
        false,
        &flag,
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
