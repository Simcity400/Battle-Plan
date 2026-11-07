//! One requested country's intel at one recorded hour, using the shared frame cache.

use dashboard_base::{
    catalogue,
    prepared::query::{frames, progressive},
    products::{Product, table_missing_reason},
    source::Source,
};

use dashboard_core::config::LoadContext;

use installed_content_core::defines::Defines;

use replay::campaign::SeekResult;
use replay_data::rows::{self, TableId, tables};
use replay_frame::frame::DecodedFrame;

use serde::{Deserialize, Serialize};

use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

use ts_rs::TS;

use super::intel_explanations;

pub struct IntelDetails;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntelDetailsParams {
    pub viewer: Option<u32>,
    pub branch_id: String,
    pub hour: i32,
    pub country: Option<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelCountry {
    pub id: u32,
    pub tag: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelDetail {
    pub key: String,
    pub group: String,
    pub label: String,
    pub viewer: u32,
    pub target: Option<u32>,
    /// All operands of this entry are evaluated at this supplying hour.
    pub supplying_hour: Option<i32>,
    pub values: Vec<IntelValue>,
    pub explanations: Vec<IntelValue>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelDetailsReply {
    pub viewer: Option<u32>,
    pub hour: i32,
    pub country: Option<u32>,
    pub countries: Vec<IntelCountry>,
    pub entries: Vec<IntelDetail>,
    pub warnings: Vec<String>,
}

pub(super) fn v(name: impl Into<String>, value: impl ToString) -> IntelValue {
    IntelValue {
        name: name.into(),
        value: value.to_string(),
    }
}

fn lanes(civilian: i64, army: i64, navy: i64, air: i64) -> Vec<IntelValue> {
    vec![
        v("Civilian", civilian),
        v("Army", army),
        v("Navy", navy),
        v("Air", air),
    ]
}

pub(super) fn detail(
    key: impl Into<String>,
    group: &str,
    label: impl Into<String>,
    viewer: u32,
    target: Option<u32>,
    values: Vec<IntelValue>,
) -> IntelDetail {
    IntelDetail {
        key: key.into(),
        group: group.into(),
        label: label.into(),
        viewer,
        target,
        supplying_hour: None,
        values,
        explanations: Vec::new(),
        unavailable: None,
    }
}

const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryIntel,
    TableId::IntelContributions,
    TableId::IntelSources,
    TableId::IntelNetworkSummaries,
    TableId::IntelNetworkAuxiliaries,
    TableId::IntelNetworks,
    TableId::PropagandaEfforts,
    TableId::WarRelations,
    TableId::FactionMembers,
    TableId::Subjects,
    TableId::Clock,
    TableId::Divisions,
    TableId::Fleets,
    TableId::TaskForces,
    TableId::Ships,
    TableId::AirWings,
    TableId::CountryFuel,
    TableId::CountryFactories,
    TableId::CountryCounterInputs,
    TableId::States,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
    TableId::TrainInventory,
    TableId::SliceReads,
];

/// The ledger's ordinary full-information predicate: self, then active-war
/// veto, then a common faction or either subject-relation endpoint direction.
/// Console fog bypass is independent and is refused only when it is reached.
pub(super) fn diplomacy_unlock(
    frame: &DecodedFrame,
    viewer_raw: u32,
    target_raw: u32,
) -> Result<Option<&'static str>, String> {
    if viewer_raw == target_raw {
        return Ok(Some("Viewer is the target country"));
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| format!("CountryLinks for ledger unlock: {e}"))?;
    let slot = |raw| {
        if raw == 0 {
            return Ok(0);
        }
        links
            .binary_search_by_key(&raw, |r| r.id)
            .ok()
            .map(|index| links[index].canonical_slot)
            .ok_or_else(|| format!("CountryLinks lacks ledger country {raw}"))
    };
    let viewer = slot(viewer_raw)?;
    let target = slot(target_raw)?;
    if viewer_raw == target_raw || viewer == target {
        return Ok(Some("Viewer is the target country"));
    }
    let wars = replay_derive::directed_war_derive::ReceiverWars::read(frame, viewer)
        .map_err(|e| format!("WarRelations for ledger unlock: {e}"))?;
    if wars.pointer_active(target) {
        return Ok(None);
    }
    let members = frame
        .complete::<tables::FactionMembers>()
        .map_err(|e| format!("FactionMembers for ledger unlock: {e}"))?;
    let mut viewer_faction = None;
    let mut target_faction = None;
    for member in members {
        let country = slot(member.country)?;
        if country == viewer {
            viewer_faction = Some(member.faction);
        }
        if country == target {
            target_faction = Some(member.faction);
        }
    }
    if viewer_faction.is_some_and(|faction| faction != 0 && Some(faction) == target_faction) {
        return Ok(Some("Same faction and no active war"));
    }
    let subjects = frame
        .complete::<tables::Subjects>()
        .map_err(|e| format!("Subjects for ledger unlock: {e}"))?;
    for subject in subjects {
        let a = slot(subject.overlord)?;
        let b = slot(subject.subject)?;
        if (a, b) == (viewer, target) || (a, b) == (target, viewer) {
            return Ok(Some("Subject relationship and no active war"));
        }
    }
    Ok(None)
}

fn project(
    frame: &DecodedFrame,
    fuel_frame: Result<&DecodedFrame, &str>,
    selected: Option<u32>,
    viewing: Option<u32>,
    defines: Result<&Defines, &str>,
    subunits: Result<&installed_content_core::unit_leader::SubUnits, &str>,
) -> io::Result<IntelDetailsReply> {
    let countries = frame.complete::<tables::Countries>()?;

    let mut choices = countries
        .iter()
        .map(|r| {
            Ok(IntelCountry {
                id: r.id,
                tag: std::str::from_utf8(&r.tag)
                    .map_err(|_| {
                        dashboard_core::invalid(format!("country {} tag is not UTF-8", r.id))
                    })?
                    .into(),
            })
        })
        .collect::<io::Result<Vec<_>>>()?;

    let reserved = frame
        .complete::<tables::IntelContributions>()
        .is_ok_and(|rows| rows.iter().any(|r| r.owner == 0))
        || frame
            .complete::<tables::IntelSources>()
            .is_ok_and(|rows| rows.iter().any(|r| r.owner == 0))
        || frame
            .complete::<tables::PropagandaEfforts>()
            .is_ok_and(|rows| rows.iter().any(|r| r.country == 0));
    if reserved && !choices.iter().any(|choice| choice.id == 0) {
        choices.push(IntelCountry {
            id: 0,
            tag: "Reserved slot 0 / raw country 0 holders".into(),
        });
    }

    let selected = selected.or_else(|| choices.first().map(|r| r.id));
    let viewer = viewing.or_else(|| countries.first().map(|r| r.id));

    let mut reply = IntelDetailsReply {
        viewer,
        hour: frame.hour,
        country: selected,
        countries: choices,
        entries: Vec::new(),
        warnings: Vec::new(),
    };

    let Some(country) = selected else {
        return Ok(reply);
    };
    let links = frame.complete::<tables::CountryLinks>().ok();

    let slot = links.and_then(|links| {
        links
            .iter()
            .find(|row| row.id == country)
            .map(|row| row.canonical_slot)
    });

    let same = |id: u32| {
        id == country
            || slot.is_some_and(|slot| {
                links.is_some_and(|links| {
                    links.iter().any(|r| r.id == id && r.canonical_slot == slot)
                })
            })
    };

    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryIntel,
        TableId::IntelContributions,
        TableId::IntelSources,
        TableId::IntelNetworkSummaries,
        TableId::IntelNetworkAuxiliaries,
        TableId::IntelNetworks,
        TableId::PropagandaEfforts,
    ] {
        if let Some(reason) = table_missing_reason(frame, table) {
            reply.warnings.push(format!("{table:?}: {reason}"));
        }
    }

    if let Ok(rows) = frame.complete::<tables::CountryIntel>() {
        for row in rows.iter().filter(|r| same(r.owner_country)) {
            reply.entries.push(detail(
                format!("intel:{}:{}", row.owner_country, row.target_index),
                "Country intel",
                "Headline intel (fixed5)",
                row.owner_country,
                links.and_then(|links| {
                    links
                        .iter()
                        .find(|r| r.canonical_slot == row.target_index)
                        .map(|r| r.id)
                }),
                lanes(row.civilian, row.army, row.navy, row.air),
            ));
        }
    }

    if let Ok(rows) = frame.complete::<tables::IntelContributions>() {
        for row in rows.iter().filter(|r| same(r.owner)) {
            let group = match row.kind {
                0 => "Allied contributions",
                1 => "Cryptology contributions",
                2 => "Static contributions",
                3 => "Dynamic contributions",
                4 => "Prior-day contributions",
                5 => "Pending contributions",
                6 => "Dynamic accumulators",
                _ => "Unknown contributions",
            };

            let header = row.ordinal == u32::MAX;

            let label = if row.pool == u32::MAX {
                format!("Pool extent: {}", row.target)
            } else if header {
                format!("Pool {}: {} entries", row.pool, row.target)
            } else {
                format!(
                    "Pool {} / entry {} / target slot {}",
                    row.pool, row.ordinal, row.target
                )
            };

            let mut entry = detail(
                format!(
                    "contrib:{}:{}:{}:{}",
                    row.owner, row.kind, row.pool, row.ordinal
                ),
                group,
                label,
                row.owner,
                None,
                if header {
                    Vec::new()
                } else {
                    lanes(row.civilian, row.army, row.navy, row.air)
                },
            );

            if row.pool != u32::MAX {
                match defines
                    .map_err(str::to_owned)
                    .and_then(|defines| intel_explanations::source(defines, row))
                {
                    Ok(values) => entry.explanations = values,
                    Err(why) => entry.unavailable = Some(why),
                }
            }

            reply.entries.push(entry);
        }
    }

    if let Ok(rows) = frame.complete::<tables::IntelSources>() {
        for row in rows.iter().filter(|r| same(r.owner)) {
            let mut values = lanes(row.civilian, row.army, row.navy, row.air);

            values.extend([
                v("Target slot", row.target),
                v("Source ID", row.id),
                v("Removed", row.removed),
                v("Next source ID", row.next_id),
                v("Discriminant", row.discriminant),
                v("Extent", row.count),
                v("Other extent", row.other_count),
                v("Reference country", row.reference_country),
                v("Reference pool", row.reference_pool),
            ]);

            reply.entries.push(detail(
                format!(
                    "source:{}:{}:{}:{}",
                    row.owner, row.pool, row.source, row.ordinal
                ),
                "Static sources and references",
                format!(
                    "Pool {} / source {} / occurrence {}",
                    row.pool, row.source, row.ordinal
                ),
                row.owner,
                None,
                values,
            ));
        }
    }

    if let Ok(rows) = frame.complete::<tables::IntelNetworkSummaries>() {
        for row in rows.iter().filter(|r| same(r.owner_country)) {
            reply.entries.push(detail(
                format!("network:{}:{}", row.owner_country, row.target_country),
                "Network summaries",
                format!("Network {}", row.ordinal),
                row.owner_country,
                Some(row.target_country),
                vec![
                    v("Covered core states", row.covered_core_states),
                    v("Covered controlled states", row.covered_controlled_states),
                    v("Covered worth", row.covered_worth),
                    v(
                        "Controlled strength sum, fixed5",
                        row.controlled_strength_sum,
                    ),
                    v("Core strength sum, fixed5", row.core_strength_sum),
                    v("Maximum state strength, fixed5", row.max_state_strength),
                ],
            ));
        }
    }

    if let Ok(rows) = frame.complete::<tables::IntelNetworkAuxiliaries>() {
        for row in rows.iter().filter(|r| same(r.owner)) {
            let names = match row.kind {
                0 if row.ordinal == 0 => [
                    "Subnetworks",
                    "Previous maximum depth",
                    "Graph rows",
                    "Unused",
                    "Unused",
                    "Unused",
                ],
                0 => [
                    "Saved local subnetwork count",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                ],
                1 => ["State", "Depth", "Unused", "Unused", "Unused", "Unused"],
                2 => [
                    "Unused",
                    "Unused",
                    "Unused",
                    "From vertex",
                    "To vertex",
                    "Unused",
                ],
                3 => [
                    "Previous source",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                ],
                4 | 16 => [
                    "Quiet flag",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                ],
                5 => [
                    "State / location",
                    "Unused",
                    "Unused",
                    "Operative handle bits",
                    "Unused",
                    "Unused",
                ],
                6 => [
                    "Operatives",
                    "Other operatives",
                    "Unused",
                    "Gain, fixed5",
                    "Detection, fixed5",
                    "Detection factor, fixed5",
                ],
                8 | 9 => [
                    "Core states",
                    "Controlled states",
                    "Worth",
                    "Unused",
                    "Unused",
                    "Unused",
                ],
                10 => [
                    "Unused",
                    "Unused",
                    "Unused",
                    "Controlled sum, fixed5",
                    "Core sum, fixed5",
                    "National coverage, fixed5",
                ],
                11 => [
                    "Unused",
                    "Unused",
                    "Unused",
                    "Detection chance, fixed5",
                    "Detection factor, fixed5",
                    "Unused",
                ],
                12 => [
                    "State",
                    "Unused",
                    "Unused",
                    "Strength, fixed5",
                    "Unused",
                    "Unused",
                ],
                13 => [
                    "Core state",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                    "Unused",
                ],
                14 => [
                    "Unused",
                    "Unused",
                    "Unused",
                    "Strength, fixed5",
                    "Gain, fixed5",
                    "Target, fixed5",
                ],
                15 => [
                    "Unused",
                    "Unused",
                    "Unused",
                    "Operative target, fixed5",
                    "Counterintelligence target, fixed5",
                    "Unused",
                ],
                _ => ["Unused"; 6],
            };

            let words = [
                i64::from(row.count_a),
                i64::from(row.count_b),
                i64::from(row.count_c),
                row.first,
                row.second,
                row.third,
            ];

            let mut values = vec![v("Country reference", row.country)];

            values.extend(
                names
                    .iter()
                    .zip(words)
                    .filter(|(n, _)| **n != "Unused")
                    .map(|(name, word)| v(*name, word)),
            );

            reply.entries.push(detail(
                format!("aux:{}:{}:{}", row.owner, row.target, row.id),
                "Network records and topology",
                format!(
                    "{} / {} / occurrence {}",
                    if row.subnetwork == u32::MAX {
                        "National".into()
                    } else {
                        format!("Subnetwork {}", row.subnetwork)
                    },
                    row.label().unwrap_or("Unknown network entry"),
                    row.ordinal
                ),
                row.owner,
                Some(row.target),
                values,
            ));
        }
    }

    if let Ok(rows) = frame.complete::<tables::IntelNetworks>() {
        for row in rows.iter().filter(|r| same(r.owner_country)) {
            let mut values = vec![
                v("Selector", row.subnetwork_selector),
                v("Local quiet", row.local_quiet),
                v("Subnetwork quiet", row.subnetwork_quiet),
                v("State strength, fixed5", row.local_strength_raw),
                v("Gain, fixed5", row.gain_raw),
                v("Operative gain, fixed5", row.gain_from_operatives_raw),
                v("Adjacent gain, fixed5", row.gain_from_adjacency_raw),
            ];

            for (name, raw) in [
                ("Subnetwork strength", row.strength_raw),
                ("Gain factor", row.gain_factor_raw),
                ("Strength target", row.strength_target_raw),
                ("Operative target", row.target_from_operatives_raw),
                (
                    "Counterintelligence target",
                    row.target_from_counterintelligence_raw,
                ),
            ] {
                values.push(v(
                    format!("{name}, fixed5"),
                    raw.map_or_else(|| "Unassigned".into(), |raw| raw.to_string()),
                ));
            }

            reply.entries.push(detail(
                format!(
                    "state:{}:{}:{}",
                    row.owner_country, row.target_country, row.state_id
                ),
                "Network states",
                format!("State {}", row.state_id),
                row.owner_country,
                Some(row.target_country),
                values,
            ));
        }
    }

    if let Ok(rows) = frame.complete::<tables::PropagandaEfforts>() {
        for row in rows.iter().filter(|r| same(r.country)) {
            let mut values = vec![
                v("Strategic operative slot", row.slot),
                v("Raw country reference", row.country),
                v("Stability, Q15", row.stability),
                v("War support, Q15", row.war_support),
            ];

            if row.sequence == u64::MAX
                && let Some((stability, war_support)) =
                    rows::PropagandaEffortRow::weekly_drift(rows, row.slot)
            {
                values = vec![
                    v("Strategic operative slot", row.slot),
                    v("Raw country reference", row.country),
                    v("Weekly stability drift, Q15", stability),
                    v("Weekly war support drift, Q15", war_support),
                ];
            }

            reply.entries.push(detail(
                format!("propaganda:{}:{}", row.slot, row.sequence),
                "Propaganda history",
                if row.sequence == u64::MAX {
                    format!("Slot {} / weekly drift", row.slot)
                } else {
                    format!("Slot {} / effort {}", row.slot, row.sequence)
                },
                row.country,
                None,
                values,
            ));
        }
    }

    if let Some(viewer) = viewer {
        reply.entries.extend(super::intel_ranges::entries(
            frame, fuel_frame, viewer, country, defines, subunits,
        ));
    }

    let mut refused: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &reply.entries {
        if let Some(reason) = entry.unavailable.as_deref() {
            *refused.entry(reason).or_default() += 1;
        }
    }
    reply.warnings.extend(
        refused
            .into_iter()
            .map(|(reason, count)| format!("{reason}: {count} reached entries")),
    );
    Ok(reply)
}

/// Read only the fuel estimate operands at the daily fact's supplying hour.
/// A current acquisition reuses the selected frame; an off-slot request shares
/// the same bounded frame cache instead of mixing retained stock with new intel.
fn fuel_frame(
    source: &Source,
    frame: &DecodedFrame,
    cancelled: &AtomicBool,
) -> Result<Box<DecodedFrame>, String> {
    let Some(hour) = frame.tables[TableId::CountryFuel.index()].sampled_at() else {
        // Untimed keyframes repeat retained rows without their read witness.
        // The shared daily-fact query finds and validates the real acquisition
        // across blocks, stopping at gaps or unavailable/inconsistent carry.
        frame
            .complete::<tables::CountryFuel>()
            .map_err(|e| format!("CountryFuel for supplying-hour recovery: {e}"))?;
        return progressive::latest_read(
            source,
            frame.branch,
            frame.hour,
            TableId::CountryFuel,
            replay_data::cadence::STAGGER_PERIOD,
            super::intel_ranges::FUEL_TABLES,
            cancelled,
        )
        .map_err(|e| format!("CountryFuel supplying-hour recovery: {e}"))?
        .ok_or_else(|| {
            "CountryFuel supplying acquisition is not recorded within the daily cadence".into()
        });
    };
    if hour > frame.hour {
        return Err("CountryFuel supplying hour is later than the queried hour".into());
    }
    if hour == frame.hour {
        return Ok(Box::new(frame.clone()));
    }
    match frames::seek(
        source,
        frame.branch,
        hour,
        super::intel_ranges::FUEL_TABLES,
        cancelled,
    )
    .map_err(|e| format!("CountryFuel supplying hour {hour}: {e}"))?
    {
        SeekResult::Frame(frame) => Ok(frame),
        _ => Err(format!(
            "CountryFuel supplying hour {hour} was not recorded"
        )),
    }
}

impl Product for IntelDetails {
    const KEY: &'static str = "intel-details";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;

    type Params = IntelDetailsParams;
    type Reply = IntelDetailsReply;

    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid intel branch")
    }

    fn types() -> Vec<String> {
        vec![
            IntelDetailsParams::decl(),
            IntelDetailsReply::decl(),
            IntelCountry::decl(),
            IntelDetail::decl(),
            IntelValue::decl(),
        ]
    }

    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let frame = match frames::seek(
            source,
            p.branch_id
                .parse::<u64>()
                .map_err(|e| dashboard_core::invalid(e.to_string()))?,
            p.hour,
            TABLES,
            cancelled,
        )? {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { reason, .. } => {
                return Err(frames::gap_error(&reason, "Intel hour was not recorded"));
            }
        };

        dashboard_text::metadata::validate_campaign(&frame, source.replay_ref())?;
        let catalogue = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options))
            .map_err(|e| e.to_string());
        let defines = catalogue
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|catalogue| {
                installed_content_core::defines::read(&catalogue.content.layers).map_err(|e| e.to_string())
            });
        let subunits = catalogue
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|catalogue| {
                installed_content_core::unit_leader::sub_units(&catalogue.content.layers)
                    .map_err(|e| e.to_string())
            });

        let fuel = fuel_frame(source, &frame, cancelled);
        project(
            &frame,
            fuel.as_deref().map_err(String::as_str),
            p.country,
            p.viewer,
            defines.as_ref().map_err(String::as_str),
            subunits.as_ref().map_err(String::as_str),
        )
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    #[test]
    fn reserved_sources_and_raw_zero_propaganda_are_selectable_with_each_slot_identity() {
        let mut frame = dashboard_core::test_fixtures::source(1, 7, None);
        *frame.tables[TableId::IntelContributions.index()] =
            TableView::Complete(TableRows::IntelContributions(vec![
                rows::IntelContributionRow {
                    owner: 0,
                    kind: 0,
                    pool: 0,
                    ordinal: u32::MAX,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::IntelSources.index()] =
            TableView::Complete(TableRows::IntelSources(vec![rows::IntelSourceRow {
                owner: 0,
                ..Default::default()
            }]));
        *frame.tables[TableId::PropagandaEfforts.index()] =
            TableView::Complete(TableRows::PropagandaEfforts(vec![
                rows::PropagandaEffortRow {
                    slot: 0,
                    country: 0,
                    sequence: 0,
                    ..Default::default()
                },
                rows::PropagandaEffortRow {
                    slot: 42,
                    country: 0,
                    sequence: 0,
                    ..Default::default()
                },
                rows::PropagandaEffortRow {
                    slot: 43,
                    country: 9001,
                    sequence: 0,
                    ..Default::default()
                },
            ]));
        let reply = project(
            &frame,
            Ok(&frame),
            Some(0),
            None,
            Err("defines missing"),
            Err("sub-unit content missing"),
        )
        .unwrap();
        assert_eq!(reply.country, Some(0));
        assert_eq!(reply.countries.iter().filter(|c| c.id == 0).count(), 1);
        assert!(
            reply
                .countries
                .iter()
                .find(|c| c.id == 0)
                .unwrap()
                .tag
                .contains("slot 0")
        );
        assert!(
            reply
                .entries
                .iter()
                .any(|e| e.group == "Allied contributions")
        );
        assert!(
            reply
                .entries
                .iter()
                .any(|e| e.group == "Static sources and references")
        );
        let history: Vec<_> = reply
            .entries
            .iter()
            .filter(|e| e.group == "Propaganda history")
            .collect();
        assert_eq!(history.len(), 2);
        for (entry, slot) in history.iter().zip([0, 42]) {
            assert!(entry.label.contains(&format!("Slot {slot} /")));
            assert!(
                entry
                    .values
                    .iter()
                    .any(|v| v.name == "Strategic operative slot" && v.value == slot.to_string())
            );
        }
        assert_ne!(history[0].key, history[1].key);
        assert!(
            !reply
                .entries
                .iter()
                .any(|e| e.key.starts_with("propaganda:43:"))
        );
    }
    #[test]

    fn intel_details_select_uncreated_countries_and_keep_ordered_history() {
        let mut frame = dashboard_core::test_fixtures::source(1, 7, None);

        *frame.tables[TableId::PropagandaEfforts.index()] =
            TableView::Complete(TableRows::PropagandaEfforts(vec![
                rows::PropagandaEffortRow {
                    country: 9001,
                    slot: 0,
                    sequence: 0,
                    stability: 32768,
                    ..Default::default()
                },
                rows::PropagandaEffortRow {
                    country: 9001,
                    slot: 0,
                    sequence: u64::MAX,
                    derived_drift: 1,
                    ..Default::default()
                },
            ]));

        let reply = project(
            &frame,
            Ok(&frame),
            Some(9001),
            None,
            Err("defines missing"),
            Err("sub-unit content missing"),
        )
        .unwrap();

        assert_eq!(reply.country, Some(9001));
        assert!(!reply.countries.is_empty());

        assert_eq!(
            reply
                .entries
                .iter()
                .filter(|e| e.group == "Propaganda history")
                .count(),
            2
        );

        assert!(reply.warnings.iter().any(|w| w.contains("IntelSources")));
    }
    #[test]
    fn off_slot_fuel_loads_the_supplying_frame_and_counts_its_missing_operands() {
        use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records};
        let hour = engine_native::cdate::HOUR_EPOCH + 1936 * 365 * 24 + 17;
        let mut supplied = dashboard_core::test_fixtures::source(1, hour, None);
        supplied.tables[TableId::CountryFuel.index()] = replay_frame::frame::SharedView::from(
            TableView::Complete(TableRows::CountryFuel(vec![rows::CountryFuelRow {
                country: 9001,
                current_raw: 1000 << 15,
                ..Default::default()
            }])),
        )
        .with_sampled_at(Some(hour));
        supplied.metrics.table_ns[TableId::CountryFuel.index()] = 1;
        let mut current = supplied.clone();
        current.hour += 1;
        current.metrics.table_ns[TableId::CountryFuel.index()] = 0;
        let replay = replay::campaign::Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![supplied, current])],
        )
        .unwrap();
        let (source, root) = file_source(replay);
        let cancelled = AtomicBool::new(false);
        let SeekResult::Frame(query) =
            frames::seek(&source, 1, hour + 1, TABLES, &cancelled).unwrap()
        else {
            panic!("query hour recorded")
        };
        assert_eq!(
            query.tables[TableId::CountryFuel.index()].sampled_at(),
            Some(hour)
        );
        let mut supplied = fuel_frame(&source, &query, &cancelled).unwrap();
        assert_eq!(supplied.hour, hour);
        let reads = frames::reads(&source);
        assert_eq!(fuel_frame(&source, &query, &cancelled).unwrap().hour, hour);
        assert_eq!(
            frames::reads(&source),
            reads,
            "supplying hour uses the shared cache"
        );
        let defines = Defines::new().unwrap();
        defines.load("synthetic.lua", br#"NDefines={NIntel={CIVILIAN_FUEL_RANGE_INTEL_MIN=0.25,CIVILIAN_FUEL_RANGE_INTEL_MAX=0.75}}"#).unwrap();
        let reply = project(
            &query,
            Ok(&supplied),
            Some(9001),
            Some(9001),
            Ok(&defines),
            Err("unused subunits"),
        )
        .unwrap();
        let fuel = reply.entries.iter().find(|r| r.label == "Fuel").unwrap();
        assert_eq!(fuel.supplying_hour, Some(hour));
        assert_eq!(fuel.values[0].value, "1000 \u{2013} 1000");
        assert!(fuel.unavailable.is_none());
        *supplied.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "supplying country aliases missing".into(),
        };
        let reply = project(
            &query,
            Ok(&supplied),
            Some(9001),
            Some(9001),
            Ok(&defines),
            Err("unused subunits"),
        )
        .unwrap();
        let fuel = reply.entries.iter().find(|r| r.label == "Fuel").unwrap();
        assert!(
            fuel.unavailable
                .as_deref()
                .unwrap()
                .contains("CountryLinks")
        );
        assert_eq!(fuel.supplying_hour, Some(hour));
        assert!(
            reply
                .warnings
                .iter()
                .any(|w| w.contains("CountryLinks for intel estimate")
                    && w.ends_with(": 1 reached entries"))
        );
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fuel_supplying_hour_recovers_across_an_untimed_carried_keyframe() {
        use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, file_source, records};
        let start = engine_native::cdate::HOUR_EPOCH + 1936 * 365 * 24;
        let make = |offset| {
            let mut frame = dashboard_core::test_fixtures::source(1, start + offset, None);
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![rows::CountryFuelRow {
                    country: 9001,
                    current_raw: 1000 << 15,
                    ..Default::default()
                }]));
            frame.metrics.table_ns[TableId::CountryFuel.index()] =
                u64::from(offset == 0 || offset == 17);
            *frame.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
                    hour: frame.hour,
                    seed: if offset <= 17 { 123 } else { 999 },
                    ..Default::default()
                }]));
            if offset > 17 {
                *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                    reason: "later aliases cannot supply historical operands".into(),
                };
            }
            frame
        };
        let replay = replay::campaign::Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, (0..24).map(make).collect())],
        )
        .unwrap();
        // A continuation starts a keyframe with carried fuel seven hours
        // after its acquisition, exactly as a periodic off-slot keyframe does.
        let replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                (24..26).map(make).collect(),
            )])
            .unwrap();
        let (source, root) = file_source(replay);
        let cancelled = AtomicBool::new(false);
        let defines = Defines::new().unwrap();
        defines.load("synthetic.lua", br#"NDefines={NIntel={CIVILIAN_FUEL_RANGE_INTEL_MIN=0.25,CIVILIAN_FUEL_RANGE_INTEL_MAX=2,CIVILIAN_FUEL_INTEL_RANGE_AT_LOWEST_INTEL=1}}"#).unwrap();
        let mut expected_frame = make(17);
        expected_frame.tables[TableId::CountryFuel.index()] = expected_frame.tables
            [TableId::CountryFuel.index()]
        .clone()
        .with_sampled_at(Some(start + 17));
        let expected = super::super::intel_ranges::entries(
            &expected_frame,
            Ok(&expected_frame),
            9001,
            9001,
            Ok(&defines),
            Err("unused subunits"),
        )
        .into_iter()
        .find(|r| r.label == "Fuel")
        .unwrap();
        for offset in [24, 25] {
            let SeekResult::Frame(query) =
                frames::seek(&source, 1, start + offset, TABLES, &cancelled).unwrap()
            else {
                panic!("query hour recorded")
            };
            assert_eq!(
                query.tables[TableId::CountryFuel.index()].sampled_at(),
                None
            );
            let supplied = fuel_frame(&source, &query, &cancelled).unwrap();
            assert_eq!(supplied.hour, start + 17);
            assert_eq!(
                supplied.tables[TableId::CountryFuel.index()].sampled_at(),
                Some(start + 17)
            );
            let reply = project(
                &query,
                Ok(&supplied),
                Some(9001),
                Some(9001),
                Ok(&defines),
                Err("unused subunits"),
            )
            .unwrap();
            let fuel = reply.entries.iter().find(|r| r.label == "Fuel").unwrap();
            assert!(fuel.unavailable.is_none(), "{:?}", fuel.unavailable);
            assert_eq!(fuel.supplying_hour, Some(start + 17));
            assert_eq!(fuel.values[0].value, expected.values[0].value);
            let reads = frames::reads(&source);
            assert_eq!(
                fuel_frame(&source, &query, &cancelled).unwrap().hour,
                start + 17
            );
            assert_eq!(
                frames::reads(&source),
                reads,
                "recovery reuses the shared daily lookup and frames"
            );
        }
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn intel_details_requests_share_one_hour_frame_cache_without_branch_scan() {
        use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records};
        let frame = dashboard_core::test_fixtures::source(1, 7, None);
        let replay =
            replay::campaign::Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])])
                .unwrap();
        let (source, root) = file_source(replay);
        let params = IntelDetailsParams {
            viewer: None,
            branch_id: "1".into(),
            hour: 7,
            country: Some(9001),
        };
        for _ in 0..2 {
            let reply = IntelDetails::load(
                &source,
                &params,
                &LoadContext { content: None },
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(reply.country, Some(9001));
        }
        assert_eq!(frames::reads(&source), (0, 1));
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ledger_unlock_uses_recorded_aliases_war_veto_faction_and_subject_context() {
        let mut frame = dashboard_core::test_fixtures::source(1, 7, None);
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                rows::CountryLinkRow {
                    id: 11,
                    canonical_slot: 7,
                },
                rows::CountryLinkRow {
                    id: 5000,
                    canonical_slot: 8,
                },
                rows::CountryLinkRow {
                    id: 9001,
                    canonical_slot: 7,
                },
            ]));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![]));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                rows::FactionMemberRow {
                    country: 5000,
                    faction: 77,
                    ordinal: 0,
                },
                rows::FactionMemberRow {
                    country: 9001,
                    faction: 77,
                    ordinal: 1,
                },
            ]));
        assert_eq!(
            diplomacy_unlock(&frame, 11, 5000).unwrap(),
            Some("Same faction and no active war")
        );
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![rows::WarRelationRow {
                first_country: 9001,
                second_country: 5000,
                ..Default::default()
            }]));
        assert_eq!(diplomacy_unlock(&frame, 11, 5000).unwrap(), None);
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![]));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![]));
        assert!(
            diplomacy_unlock(&frame, 11, 5000)
                .unwrap_err()
                .contains("Subjects")
        );
        *frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![rows::SubjectRow {
                overlord: 9001,
                subject: 5000,
                ..Default::default()
            }]));
        assert_eq!(
            diplomacy_unlock(&frame, 11, 5000).unwrap(),
            Some("Subject relationship and no active war")
        );
        assert_eq!(
            diplomacy_unlock(&frame, 11, 9001).unwrap(),
            Some("Viewer is the target country")
        );
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "denied".into(),
        };
        assert!(
            diplomacy_unlock(&frame, 11, 5000)
                .unwrap_err()
                .contains("CountryLinks")
        );
    }
}
