//! Exact observed aircraft bookkeeping and applied bombing health loss.
//! AirLossEvents keeps the writer context because its ordinal families
//! overlap (engine::air_loss_sites). Never sum the daily field array. Bombing
//! includes only proved air-mission and air-naval callers of building damage;
//! other callers are not aircraft bombing. A complete interval certifies an
//! empty event list. Gaps and unavailable intervals certify nothing.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_groups::HourSpan};
use crate::{
    Source, catalogue,
    metadata::{branch_bounds, check_cancel, tagged_countries, validate_campaign},
    ranking,
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{BuildingBombingDamageCause, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct MilitaryAirLedger;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAirFamily {
    AirLosses,
    Bombing,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum MilitaryAirMetric {
    Dealt,
    Taken,
    Total,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirTop {
    pub count: usize,
    pub metric: MilitaryAirMetric,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirParams {
    pub branch_id: String,
    pub family: MilitaryAirFamily,
    #[serde(default)]
    pub regions: Vec<u32>,
    #[serde(default)]
    pub causes: Vec<u32>,
    #[serde(default)]
    /// A range starts at native application time; publication still gates visibility.
    pub from_hour: Option<i32>,
    #[serde(default)]
    pub top: Option<MilitaryAirTop>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirCountry {
    pub tag: String,
    /// Native canonical slot and country ID; tag changes do not join holders.
    pub identity: String,
    pub present: Vec<HourSpan>,
    /// Null means no ranking filter. Empty means never in the requested top N.
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirPoint {
    /// Native application time, not the later publication time.
    pub hour: i32,
    pub read_hour: i32,
    /// Exact sum of observed signed increments through this point, in native units.
    pub total_raw: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirLeaf {
    /// Null means the recording could not attribute this endpoint.
    pub tag: Option<String>,
    pub identity: Option<String>,
    pub dealt: bool,
    /// Opposite endpoint from the native wrapper, resolved at the supplying read.
    /// Accident losses are operational; a null opponent for another cause is unknown.
    pub opponent: Option<String>,
    pub region: Option<u32>,
    pub building_token: Option<String>,
    pub building_id: Option<u32>,
    /// Zero is known absence of a containing state, not an inferred state.
    pub state: Option<u32>,
    pub cause: u32,
    pub points: Vec<MilitaryAirPoint>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirName {
    pub id: String,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirReply {
    pub family: MilitaryAirFamily,
    pub first_hour: i32,
    pub last_hour: i32,
    pub available: bool,
    pub observed: Vec<HourSpan>,
    /// Reads containing a caller whose membership in aircraft bombing is unknown.
    pub incomplete: Vec<HourSpan>,
    pub countries: Vec<MilitaryAirCountry>,
    pub leaves: Vec<MilitaryAirLeaf>,
    pub regions: Vec<MilitaryAirName>,
    pub buildings: Vec<MilitaryAirName>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LeafKey {
    country: Option<(String, String)>,
    dealt: bool,
    opponent: Option<String>,
    region: Option<u32>,
    building: Option<(u32, Option<String>)>,
    state: Option<u32>,
    cause: u32,
}

#[derive(Default)]
struct FamilyScan {
    observed: Vec<HourSpan>,
    incomplete: Vec<HourSpan>,
    countries: BTreeMap<(String, String), Vec<HourSpan>>,
    /// Publish in read order; native dates never backfill a not-yet-recorded event.
    leaves: BTreeMap<LeafKey, BTreeMap<(i32, i32), i128>>,
    warnings: BTreeMap<String, usize>,
    last_event: Option<u64>,
}

impl FamilyScan {
    fn note(&mut self, reason: impl Into<String>) {
        *self.warnings.entry(reason.into()).or_default() += 1;
    }

    fn event(&mut self, id: u64) -> io::Result<()> {
        if self.last_event.is_some_and(|last| id <= last) {
            return Err(crate::invalid(
                "military air event ordinal repeated or moved backwards",
            ));
        }
        self.last_event = Some(id);
        Ok(())
    }

    fn add(&mut self, key: LeafKey, hour: i32, read_hour: i32, amount: i64) -> io::Result<()> {
        let entry = self
            .leaves
            .entry(key)
            .or_default()
            .entry((read_hour, hour))
            .or_default();
        *entry = entry
            .checked_add(i128::from(amount))
            .ok_or_else(|| crate::invalid("military air event sum overflow"))?;
        Ok(())
    }

    fn identity_read(
        &mut self,
        frame: &DecodedFrame,
    ) -> io::Result<BTreeMap<u32, (String, String)>> {
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        let mut slots = BTreeMap::new();
        for (tag, lookup) in tagged_countries(countries, links) {
            let Some((country, slot)) = lookup.found() else {
                continue;
            };
            let Ok(tag) = std::str::from_utf8(tag) else {
                self.note("country tag is not UTF-8; its events cannot be named");
                continue;
            };
            super::validate_tag(tag).map_err(crate::invalid)?;
            let key = (tag.to_owned(), format!("{slot}:{}", country.id));
            append_hour(self.countries.entry(key.clone()).or_default(), frame.hour);
            slots.insert(slot, key);
        }
        Ok(links
            .iter()
            .filter_map(|link| {
                slots
                    .get(&link.canonical_slot)
                    .cloned()
                    .map(|country| (link.id, country))
            })
            .collect())
    }
}

fn append_hour(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(last) = spans.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
    {
        last.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

fn event_country(
    raw: Option<u32>,
    countries: &BTreeMap<u32, (String, String)>,
) -> Option<(String, String)> {
    raw.filter(|raw| *raw != 0)
        .and_then(|raw| countries.get(&raw))
        .cloned()
}

fn air_frame(scan: &mut FamilyScan, frame: &DecodedFrame) -> io::Result<()> {
    let rows = match frame.complete::<tables::AirLossEvents>() {
        Ok(rows) => rows,
        Err(_) => return Ok(()),
    };
    let countries = match scan.identity_read(frame) {
        Ok(countries) => countries,
        Err(_) => return Ok(()),
    };
    append_hour(&mut scan.observed, frame.hour);
    for row in rows {
        scan.event(row.id)?;
        if row.game_hour > frame.hour {
            return Err(crate::invalid(
                "air event is dated after its supplying read",
            ));
        }
        let country = event_country(Some(row.country), &countries);
        if country.is_none() {
            scan.note("air event endpoint is not resolved by its recorded country census");
        }
        if !(1..=6).contains(&row.native_cause) {
            scan.note(format!(
                "air loss cause {} has no proved display meaning; its exact amount is retained",
                row.native_cause
            ));
        }
        let opponent = event_country(row.opponent, &countries).map(|country| country.0);
        scan.add(
            LeafKey {
                country,
                dealt: row.credited_enemy,
                opponent,
                region: Some(row.region),
                building: None,
                state: None,
                cause: row.native_cause,
            },
            row.game_hour,
            frame.hour,
            row.amount_raw,
        )?;
    }
    Ok(())
}

fn bombing_frame(scan: &mut FamilyScan, frame: &DecodedFrame) -> io::Result<()> {
    let rows = match frame.complete::<tables::BuildingBombingDamage>() {
        Ok(rows) => rows,
        Err(_) => return Ok(()),
    };
    let countries = match scan.identity_read(frame) {
        Ok(countries) => countries,
        Err(_) => return Ok(()),
    };
    append_hour(&mut scan.observed, frame.hour);
    if rows
        .iter()
        .any(|row| row.cause == BuildingBombingDamageCause::Unknown)
    {
        append_hour(&mut scan.incomplete, frame.hour);
    }
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    for row in rows {
        scan.event(row.id)?;
        if row.cause == BuildingBombingDamageCause::Unknown {
            scan.note("building damage has an unsupported caller; its contribution to aircraft bombing is not known");
            continue;
        }
        if !matches!(
            row.cause,
            BuildingBombingDamageCause::AirMission | BuildingBombingDamageCause::AirNaval
        ) {
            continue;
        }
        if row.game_hour > frame.hour {
            return Err(crate::invalid(
                "bombing event is dated after its supplying read",
            ));
        }
        let token = lexical
            .and_then(|rows| {
                rows.binary_search_by_key(&row.building_type, |row| row.token)
                    .ok()
                    .and_then(|at| std::str::from_utf8(&rows[at].literal).ok())
            })
            .filter(|token| !token.is_empty())
            .map(str::to_owned);
        if token.is_none() {
            scan.note("building type is not spelled by the event's recorded lexical dictionary");
        }
        for (dealt, raw) in [(true, row.attacker), (false, Some(row.victim))] {
            let country = event_country(raw, &countries);
            if country.is_none() {
                scan.note(if dealt {
                    "bombing attacker not recorded"
                } else {
                    "bombing victim not recorded"
                });
            }
            scan.add(
                LeafKey {
                    country,
                    dealt,
                    opponent: None,
                    region: None,
                    building: Some((row.building_type, token.clone())),
                    state: Some(row.state),
                    cause: row.cause as u32,
                },
                row.game_hour,
                frame.hour,
                row.amount_raw,
            )?;
        }
    }
    Ok(())
}

struct Scan {
    first: i32,
    last: i32,
    air: FamilyScan,
    bombing: FamilyScan,
}

fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let declaration = replay
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| crate::invalid("Unknown branch"))?;
    let (first, last) = branch_bounds(replay, declaration);
    let mut scan = Scan {
        first,
        last,
        air: FamilyScan::default(),
        bombing: FamilyScan::default(),
    };
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::AirLossEvents,
        TableId::BuildingBombingDamage,
    ];
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = replay.read_block_selected(block, &selected)?;
        for frame in frames {
            check_cancel(cancelled)?;
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, replay)?;
            air_frame(&mut scan.air, decoded)?;
            bombing_frame(&mut scan.bombing, decoded)?;
        }
    }
    Ok(scan)
}

type ScanCache = Vec<(u64, Arc<Scan>)>;

fn scanned(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scan>> {
    check_cancel(cancelled)?;
    let state = source.state::<ScanCache>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, scan)) = state.iter().find(|(id, _)| *id == branch) {
        return Ok(Arc::clone(scan));
    }
    // Concurrent widget requests share the same selected-table branch scan.
    let scan = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
    state.retain(|(id, _)| *id != branch);
    state.insert(0, (branch, Arc::clone(&scan)));
    state.truncate(2); // Retained branches, never a recording/data count limit.
    Ok(scan)
}

fn leaves(scan: &FamilyScan, from_hour: Option<i32>) -> io::Result<Vec<MilitaryAirLeaf>> {
    scan.leaves
        .iter()
        .map(|(key, deltas)| {
            let mut total = 0_i128;
            let points = deltas
                .iter()
                .filter(|((_, hour), _)| from_hour.is_none_or(|from| *hour >= from))
                .map(|(&(read_hour, hour), &amount)| {
                    total = total
                        .checked_add(amount)
                        .ok_or_else(|| crate::invalid("military air prefix sum overflow"))?;
                    Ok(MilitaryAirPoint {
                        hour,
                        read_hour,
                        total_raw: total.to_string(),
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            Ok(MilitaryAirLeaf {
                tag: key.country.as_ref().map(|country| country.0.clone()),
                identity: key.country.as_ref().map(|country| country.1.clone()),
                dealt: key.dealt,
                opponent: key.opponent.clone(),
                region: key.region,
                building_token: key
                    .building
                    .as_ref()
                    .and_then(|building| building.1.clone()),
                building_id: key.building.as_ref().map(|building| building.0),
                state: key.state,
                cause: key.cause,
                points,
            })
        })
        .collect()
}

struct RankRun {
    span: HourSpan,
    value: i128,
}

fn ranked_membership(
    countries: &BTreeMap<(String, String), Vec<HourSpan>>,
    leaves: &[MilitaryAirLeaf],
    from: i32,
    top: &MilitaryAirTop,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let mut deltas: BTreeMap<&str, BTreeMap<i32, i128>> = BTreeMap::new();
    for leaf in leaves {
        let Some(identity) = leaf.identity.as_deref() else {
            continue;
        };
        let included = match top.metric {
            MilitaryAirMetric::Dealt => leaf.dealt,
            MilitaryAirMetric::Taken => !leaf.dealt,
            MilitaryAirMetric::Total => true,
        };
        if !included {
            continue;
        }
        let mut before = 0_i128;
        for point in &leaf.points {
            let total = point
                .total_raw
                .parse::<i128>()
                .map_err(|_| crate::invalid("invalid military air prefix"))?;
            let delta = total
                .checked_sub(before)
                .ok_or_else(|| crate::invalid("military air ranking delta overflow"))?;
            before = total;
            if point.read_hour < from {
                continue;
            }
            let value = deltas
                .entry(identity)
                .or_default()
                .entry(point.read_hour)
                .or_default();
            *value = value
                .checked_add(delta)
                .ok_or_else(|| crate::invalid("military air ranking sum overflow"))?;
        }
    }
    for values in deltas.values_mut() {
        let mut total = 0_i128;
        for value in values.values_mut() {
            total = total
                .checked_add(*value)
                .ok_or_else(|| crate::invalid("military air ranking prefix overflow"))?;
            *value = total;
        }
    }
    let mut series: BTreeMap<String, Vec<RankRun>> = BTreeMap::new();
    for ((tag, identity), spans) in countries {
        check_cancel(cancelled)?;
        let values = deltas.get(identity.as_str());
        for span in spans {
            let first = span.first_hour.max(from);
            if first > span.last_hour {
                continue;
            }
            let mut changes = vec![(
                first,
                values
                    .and_then(|values| values.range(..=first).next_back())
                    .map_or(0, |(_, value)| *value),
            )];
            if let Some(values) = values {
                changes.extend(
                    values
                        .range((
                            std::ops::Bound::Excluded(first),
                            std::ops::Bound::Included(span.last_hour),
                        ))
                        .map(|(&hour, &value)| (hour, value)),
                );
            }
            for (index, &(first_hour, value)) in changes.iter().enumerate() {
                let last_hour = changes
                    .get(index + 1)
                    .map_or(span.last_hour, |(hour, _)| hour - 1);
                series.entry(tag.clone()).or_default().push(RankRun {
                    span: HourSpan {
                        first_hour,
                        last_hour,
                    },
                    value,
                });
            }
        }
    }
    for runs in series.values_mut() {
        runs.sort_by_key(|run| run.span.first_hour);
    }
    ranking::top_membership_exact(
        &series,
        top.count,
        |run| (run.span.first_hour, run.span.last_hour),
        |run| Some(run.value),
        true,
        cancelled,
    )
}

fn names(
    source: &Source,
    context: &LoadContext,
    scan: &FamilyScan,
    warnings: &mut Vec<String>,
) -> (Vec<MilitaryAirName>, Vec<MilitaryAirName>) {
    let cat = context
        .content()
        .and_then(|options| catalogue::catalogue(source, options));
    let regions: BTreeSet<_> = scan.leaves.keys().filter_map(|key| key.region).collect();
    let buildings: BTreeSet<_> = scan
        .leaves
        .keys()
        .filter_map(|key| {
            key.building
                .as_ref()
                .and_then(|building| building.1.clone())
        })
        .collect();
    let named = |id: String, result: Result<String, String>| match result {
        Ok(name) => MilitaryAirName {
            id,
            name: Some(name),
            missing: None,
        },
        Err(reason) => MilitaryAirName {
            id,
            name: None,
            missing: Some(reason),
        },
    };
    let Ok(cat) = cat else {
        let reason =
            "Recorded content stack unavailable; choose matching game and user folders".to_owned();
        warnings.push(reason.clone());
        return (
            regions
                .into_iter()
                .map(|id| named(id.to_string(), Err(reason.clone())))
                .collect(),
            buildings
                .into_iter()
                .map(|id| named(id, Err(reason.clone())))
                .collect(),
        );
    };
    warnings.extend(cat.verification.iter().cloned());
    warnings.extend(cat.warnings.iter().cloned());
    let regions = if regions.is_empty() {
        Vec::new()
    } else {
        let content =
            installed_content::regions::read(&cat.content.layers).map_err(|error| error.to_string());
        if let Ok(content) = &content {
            warnings.extend(content.notes.iter().cloned());
        }
        regions
            .into_iter()
            .map(|id| {
                named(
                    id.to_string(),
                    content
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|content| region_name(&cat, content, id)),
                )
            })
            .collect()
    };
    let buildings = if buildings.is_empty() {
        Vec::new()
    } else {
        let content =
            installed_content::buildings::read(&cat.content.layers).map_err(|error| error.to_string());
        buildings
            .into_iter()
            .map(|id| {
                let name = content
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|content| building_name(&cat, content, &id));
                named(id, name)
            })
            .collect()
    };
    (regions, buildings)
}

fn region_name(
    cat: &catalogue::Catalogue,
    content: &installed_content::regions::StrategicRegions,
    id: u32,
) -> Result<String, String> {
    let region = content
        .regions
        .get(&id)
        .ok_or_else(|| format!("Region {id} absent from the recorded stack"))?;
    cat.localised(&region.name_key, catalogue::View::Plain)
        .ok_or_else(|| format!("Region {id} name unsupported"))
}

fn building_name(
    cat: &catalogue::Catalogue,
    content: &installed_content::buildings::Buildings,
    token: &str,
) -> Result<String, String> {
    let building = content
        .by_token(token)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Building {token} absent from the recorded stack"))?;
    cat.localised(&building.name, catalogue::View::Plain)
        .ok_or_else(|| format!("Building {token} name unsupported"))
}

/// The same native content readers and naming path as recorded event selections.
pub(crate) fn check_content(
    cat: &catalogue::Catalogue,
) -> io::Result<Vec<crate::content_check::Finding>> {
    use crate::content_check::{Finding, Kind};
    let regions = installed_content::regions::read(&cat.content.layers)?;
    let buildings = installed_content::buildings::read(&cat.content.layers)?;
    let mut findings = Vec::new();
    for id in regions.regions.keys() {
        if let Err(reason) = region_name(cat, &regions, *id) {
            findings.push(Finding::new(
                Kind::Refused,
                format!("air region {id}"),
                reason,
            ));
        }
    }
    for token in buildings.types.keys() {
        if let Err(reason) = building_name(cat, &buildings, token) {
            findings.push(Finding::new(
                Kind::Refused,
                format!("bombing building {token}"),
                reason,
            ));
        }
    }
    findings.extend(cat.take_findings());
    Ok(findings)
}

impl Product for MilitaryAirLedger {
    const KEY: &'static str = "military-air-ledger";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MilitaryAirParams;
    type Reply = MilitaryAirReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params.top.as_ref().is_some_and(|top| top.count == 0) {
            return Err("Top count must be positive");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            MilitaryAirParams::decl(),
            MilitaryAirFamily::decl(),
            MilitaryAirMetric::decl(),
            MilitaryAirTop::decl(),
            MilitaryAirCountry::decl(),
            MilitaryAirPoint::decl(),
            MilitaryAirLeaf::decl(),
            MilitaryAirName::decl(),
            MilitaryAirReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scan = scanned(
            source,
            params.branch_id.parse().expect("validated branch"),
            cancelled,
        )?;
        let family = match params.family {
            MilitaryAirFamily::AirLosses => &scan.air,
            MilitaryAirFamily::Bombing => &scan.bombing,
        };
        let selected_leaves: Vec<_> = leaves(family, params.from_hour)?
            .into_iter()
            .filter(|leaf| {
                (params.regions.is_empty()
                    || leaf
                        .region
                        .is_some_and(|region| params.regions.contains(&region)))
                    && (params.causes.is_empty() || params.causes.contains(&leaf.cause))
            })
            .collect();
        let membership = params
            .top
            .as_ref()
            .map(|top| {
                ranked_membership(
                    &family.countries,
                    &selected_leaves,
                    params.from_hour.unwrap_or_else(|| {
                        family
                            .observed
                            .first()
                            .map_or(scan.first, |span| span.first_hour)
                    }),
                    top,
                    cancelled,
                )
            })
            .transpose()?;
        let mut warnings: Vec<_> = family
            .warnings
            .iter()
            .map(|(reason, count)| format!("{count} events: {reason}"))
            .collect();
        let (regions, buildings) = names(source, context, family, &mut warnings);
        let countries = family
            .countries
            .iter()
            .map(|((tag, identity), present)| {
                let selected = membership
                    .as_ref()
                    .map(|membership| {
                        let mut selected = Vec::new();
                        ranking::for_each_piece(
                            present,
                            membership.get(tag).map_or(&[], Vec::as_slice),
                            |span| (span.first_hour, span.last_hour),
                            cancelled,
                            |_, first_hour, last_hour| {
                                selected.push(HourSpan {
                                    first_hour,
                                    last_hour,
                                });
                                Ok(())
                            },
                        )?;
                        Ok::<_, io::Error>(selected)
                    })
                    .transpose()?;
                Ok(MilitaryAirCountry {
                    tag: tag.clone(),
                    identity: identity.clone(),
                    present: present.clone(),
                    membership: selected,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let reply = MilitaryAirReply {
            family: params.family,
            first_hour: scan.first,
            last_hour: scan.last,
            available: !family.observed.is_empty(),
            observed: family.observed.clone(),
            incomplete: family.incomplete.clone(),
            countries,
            leaves: selected_leaves,
            regions,
            buildings,
            warnings,
        };
        check_cancel(cancelled)?;
        if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(crate::invalid(
                "Military air ledger reply exceeds transport budget; nothing was truncated",
            ));
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
