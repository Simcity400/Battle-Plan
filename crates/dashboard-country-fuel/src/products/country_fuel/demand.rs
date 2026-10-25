//! Current required-use rates for the fuel-panel breakdown, independent of
//! completed updates and receipts cached by divisions and task forces.
//! Raw arithmetic/topology belongs to the supplying FuelDemandRead.
use super::*;
use replay_data::{
    rows::FuelDemandRead, rows::FuelDemandSite, rows::FuelDemandUnit, rows::FuelDemandWing,
};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelConsumptionRead {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub segment_start_hour: i32,
    pub country: String,
    pub country_instance: Option<u32>,
    /// Native family sum then native daily conversion, including signed wrap.
    pub native_daily_raw: [Option<String>; 3],
    /// Widened sum of individually converted known rows; never the native sum.
    pub known_daily_raw: [String; 3],
    pub positive_daily_raw: [String; 3],
    pub reasons: [Option<String>; 3],
}
/// A [`FuelConsumptionRead`] as exact integers; only replies format them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawConsumption {
    pub(super) first_hour: i32,
    pub(super) last_hour: i32,
    starts_segment: bool,
    pub(super) segment_start_hour: i32,
    country: u32,
    country_instance: Option<u32>,
    /// Each family's native daily sum, or why it is unknown.
    native_daily: [Result<i64, Arc<str>>; 3],
    known_daily: [i128; 3],
    positive_daily: [i128; 3],
}
impl From<&RawConsumption> for FuelConsumptionRead {
    fn from(read: &RawConsumption) -> Self {
        Self {
            first_hour: read.first_hour,
            last_hour: read.last_hour,
            starts_segment: read.starts_segment,
            segment_start_hour: read.segment_start_hour,
            country: read.country.to_string(),
            country_instance: read.country_instance,
            native_daily_raw: read
                .native_daily
                .each_ref()
                .map(|daily| daily.as_ref().ok().map(i64::to_string)),
            known_daily_raw: read.known_daily.map(|value| value.to_string()),
            positive_daily_raw: read.positive_daily.map(|value| value.to_string()),
            reasons: read
                .native_daily
                .each_ref()
                .map(|daily| daily.as_ref().err().map(|reason| reason.to_string())),
        }
    }
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelConsumptionReply {
    pub branch_id: String,
    pub tag: String,
    pub reads: Vec<FuelConsumptionRead>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandPart {
    pub key: String,
    pub kind: u32,
    pub reference: String,
    pub parent: String,
    pub ordinal: u32,
    pub owners: Option<[String; 3]>,
    pub wing_owner: Option<String>,
    pub group: Option<String>,
    pub topology_reason: Option<String>,
    pub site: Option<FuelDemandLocation>,
    pub amount_raw: Option<String>,
    pub reason: Option<String>,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandLocation {
    pub list: u8,
    pub site_ordinal: u32,
    pub base: String,
    pub container_owner: Option<String>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandCommand {
    pub reference: String,
    pub parent: String,
    pub country: String,
    pub army_group: bool,
    pub members: Vec<String>,
    pub children: Vec<String>,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandFleet {
    pub reference: String,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandAirSite {
    pub list: u8,
    pub ordinal: u32,
    pub base: String,
    pub container_owner: Option<String>,
    pub capacity_raw: Option<String>,
    pub demilitarized: Option<u8>,
    pub reason: Option<String>,
    pub wings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub read: FuelConsumptionRead,
    pub parts: Vec<FuelDemandPart>,
    pub commands: Vec<FuelDemandCommand>,
    pub fleets: Vec<FuelDemandFleet>,
    pub sites: Vec<FuelDemandAirSite>,
    pub topology_reason: Option<String>,
    pub warnings: Vec<String>,
}

// Positions preserve repeated native occurrences. Full identity/topology words
// certify that the same position in another read still denotes this occurrence.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Unit(u32, usize, u64, u64, u32, Option<[u32; 3]>),
    Wing(
        usize,
        usize,
        u8,
        u32,
        u64,
        u32,
        u32,
        u64,
        Option<u32>,
        Option<u32>,
    ),
}
struct Occurrence<'a> {
    key: Key,
    kind: u32,
    reference: u64,
    parent: u64,
    ordinal: u32,
    owners: Option<[u32; 3]>,
    wing_owner: Option<u32>,
    group: Option<u64>,
    topology_reason: &'a [u8],
    site: Option<FuelDemandLocation>,
    rate: Result<i64, String>,
    name: Option<&'a [u8]>,
    name_reason: &'a [u8],
}
fn reason(bytes: &[u8]) -> Option<String> {
    (!bytes.is_empty()).then(|| {
        std::str::from_utf8(bytes)
            .unwrap_or("Fuel refusal text is not UTF-8")
            .to_owned()
    })
}
fn pack_of(row: &CountryFuelRow, hour: i32) -> Result<&FuelDemandRead, String> {
    let replay_data::rows::Recorded::Value(pack) = &row.demand else {
        return Err("Individual current fuel consumption was not recorded".into());
    };
    // A pack read with its row was read at the row's read hour, `hour`.
    if pack.hour.at(hour) != hour {
        return Err(
            "Current fuel consumption supplying read or native country identity does not match"
                .into(),
        );
    }
    if let Some(reason) = reason(&pack.reason) {
        return Err(reason);
    }
    Ok(pack)
}
fn unit(kind: u32, index: usize, row: &FuelDemandUnit) -> Occurrence<'_> {
    Occurrence {
        key: Key::Unit(
            kind,
            index,
            row.reference,
            row.parent,
            row.ordinal,
            row.owners,
        ),
        kind,
        reference: row.reference,
        parent: row.parent,
        ordinal: row.ordinal,
        owners: row.owners,
        wing_owner: None,
        group: row.group,
        topology_reason: &row.group_reason,
        site: None,
        rate: row.rate_raw.ok_or_else(|| {
            reason(&row.reason)
                .unwrap_or_else(|| "Current cached fuel rate was not recorded".into())
        }),
        name: row.name.as_deref(),
        name_reason: &row.name_reason,
    }
}
fn wing(pack: &FuelDemandRead, site_index: usize, index: usize) -> Option<Occurrence<'_>> {
    let site = pack.sites.get(site_index)?;
    let row = site.wings.get(index)?;
    let rate = if let Some(reason) = reason(&row.reason) {
        Err(reason)
    } else {
        pack.wing_rate(row).map_err(str::to_owned)
    };
    Some(Occurrence {
        key: wing_key(site_index, index, site, row),
        kind: 1,
        reference: row.reference,
        parent: site.base,
        ordinal: row.wing_ordinal,
        owners: None,
        wing_owner: row.owner,
        group: None,
        topology_reason: &row.owner_reason,
        site: Some(FuelDemandLocation {
            list: site.list,
            site_ordinal: site.ordinal,
            base: site.base.to_string(),
            container_owner: site.container_owner.map(|owner| owner.to_string()),
            pool_ordinal: row.pool_ordinal,
            wing_ordinal: row.wing_ordinal,
            reason: reason(&site.reason),
        }),
        rate,
        name: row.name.as_deref(),
        name_reason: &row.name_reason,
    })
}
fn wing_key(site_index: usize, index: usize, site: &FuelDemandSite, row: &FuelDemandWing) -> Key {
    Key::Wing(
        site_index,
        index,
        site.list,
        site.ordinal,
        site.base,
        row.pool_ordinal,
        row.wing_ordinal,
        row.reference,
        row.owner,
        site.container_owner,
    )
}
fn occurrences(pack: &FuelDemandRead) -> impl Iterator<Item = Occurrence<'_>> {
    pack.divisions
        .iter()
        .enumerate()
        .map(|(index, row)| unit(0, index, row))
        .chain(
            pack.task_forces
                .iter()
                .enumerate()
                .map(|(index, row)| unit(2, index, row)),
        )
        .chain(pack.sites.iter().enumerate().flat_map(move |(site, item)| {
            (0..item.wings.len())
                .map(move |index| wing(pack, site, index).expect("enumerated fuel wing"))
        }))
}
fn occurrence<'a>(pack: &'a FuelDemandRead, key: &Key) -> Option<Occurrence<'a>> {
    let row = match key {
        Key::Unit(0, index, ..) => unit(0, *index, pack.divisions.get(*index)?),
        Key::Unit(2, index, ..) => unit(2, *index, pack.task_forces.get(*index)?),
        Key::Wing(site, index, ..) => wing(pack, *site, *index)?,
        _ => return None,
    };
    (&row.key == key).then_some(row)
}

pub(super) fn summary_of(
    row: &CountryFuelRow,
    hour: i32,
    segment: i32,
    starts: bool,
    reasons: &mut Reasons,
) -> RawConsumption {
    let mut known = [0_i128; 3];
    let mut positive = [0_i128; 3];
    let native_daily = match pack_of(row, hour) {
        Err(reason) => {
            let reason = reasons.intern(&reason);
            std::array::from_fn(|_| Err(Arc::clone(&reason)))
        }
        Ok(pack) => {
            // native_rates orders Army/Navy/Air; the established Fuel consumers order
            // Army/Air/Navy. No new arithmetic law lives in the product.
            let rates = pack.native_rates();
            let rates = [rates[0], rates[2], rates[1]];
            let causes = [&pack.army_reason, &pack.air_reason, &pack.navy_reason];
            let native = std::array::from_fn(|index| {
                rates[index]
                    .map(engine_fuel::fuel_demand::daily_required)
                    .map_err(|message| {
                        reasons.intern(&reason(causes[index]).unwrap_or_else(|| message.into()))
                    })
            });
            for row in occurrences(pack) {
                if let Ok(rate) = row.rate {
                    let value = i128::from(engine_fuel::fuel_demand::daily_required(rate));
                    known[row.kind as usize] += value;
                    positive[row.kind as usize] += value.max(0);
                }
            }
            native
        }
    };
    RawConsumption {
        first_hour: hour,
        last_hour: hour,
        starts_segment: starts,
        segment_start_hour: segment,
        country: row.country,
        country_instance: match row.country_instance {
            replay_data::rows::Recorded::Value(value) => value,
            _ => None,
        },
        native_daily,
        known_daily: known,
        positive_daily: positive,
    }
}

fn stored_name(
    name: Option<&[u8]>,
    refused: &[u8],
    format: &mut impl FnMut(&[u8]) -> Result<String, String>,
) -> (Option<String>, Option<String>) {
    if let Some(reason) = reason(refused) {
        return (None, Some(reason));
    }
    match name {
        Some(bytes) => match format(bytes) {
            Ok(name) => (Some(name), None),
            Err(reason) => (None, Some(reason)),
        },
        None => (
            None,
            Some("The native supplying-read name was not recorded".into()),
        ),
    }
}
fn read_name(
    source: &Source,
    context: &LoadContext,
    cache: &std::sync::OnceLock<io::Result<Arc<dashboard_text::catalogue::Catalogue>>>,
    bytes: &[u8],
) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map_err(|_| "fuel current consumption cached name is not UTF-8".to_string())?;
    if bytes.is_empty() {
        return Ok(String::new());
    }
    let catalogue = cache
        .get_or_init(|| {
            context
                .content()
                .and_then(|options| dashboard_base::catalogue::catalogue(source, options))
        })
        .as_ref()
        .map_err(|error| error.to_string())?;
    dashboard_text::scripted_text::stored_text(
        &catalogue.localisation,
        &catalogue.recorded_language,
        bytes,
        "fuel current consumption cached name",
    )
    .and_then(|text| dashboard_text::catalogue::display_text(text.as_bytes()))
}
fn part_of(
    row: Occurrence<'_>,
    format: &mut impl FnMut(&[u8]) -> Result<String, String>,
) -> io::Result<FuelDemandPart> {
    let (recorded_name, name_reason) = stored_name(row.name, row.name_reason, format);
    let (amount_raw, quantity_reason) = match row.rate {
        Ok(rate) => (
            Some(engine_fuel::fuel_demand::daily_required(rate).to_string()),
            None,
        ),
        Err(reason) => (None, Some(reason)),
    };
    Ok(FuelDemandPart {
        key: serde_json::to_string(&row.key)
            .map_err(|error| dashboard_core::invalid(error.to_string()))?,
        kind: row.kind,
        reference: row.reference.to_string(),
        parent: row.parent.to_string(),
        ordinal: row.ordinal,
        owners: row
            .owners
            .map(|owners| owners.map(|owner| owner.to_string())),
        wing_owner: row.wing_owner.map(|owner| owner.to_string()),
        group: row.group.map(|group| group.to_string()),
        topology_reason: reason(row.topology_reason),
        site: row.site,
        amount_raw,
        reason: quantity_reason,
        recorded_name,
        name_reason,
    })
}
fn warnings(reasons: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut counts = BTreeMap::new();
    for reason in reasons {
        *counts.entry(reason).or_insert(0_usize) += 1;
    }
    counts
        .into_iter()
        .map(|(reason, count)| {
            format!("{count} current fuel consumption operands refused: {reason}")
        })
        .collect()
}
#[cfg(any(test, feature = "test-support"))]
pub(super) fn summary_at<'a>(
    branch: &'a ScannedBranch,
    params: &FuelAwardsParams,
) -> io::Result<&'a RawConsumption> {
    branch
        .countries
        .get(&params.tag)
        .and_then(|country| {
            country
                .consumption
                .binary_search_by_key(&params.hour, |read| read.first_hour)
                .ok()
                .map(|index| &country.consumption[index])
        })
        .filter(|read| {
            read.country.to_string() == params.country
                && read.country_instance == Some(params.country_instance)
        })
        .ok_or_else(|| {
            dashboard_core::invalid(
                "Current fuel consumption actor or supplying read was not recorded",
            )
        })
}

pub struct CountryFuelConsumption;
impl Product for CountryFuelConsumption {
    const KEY: &'static str = "country-fuel-consumption";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CountryParams;
    type Reply = FuelConsumptionReply;
    fn types() -> Vec<String> {
        vec![FuelConsumptionRead::decl(), FuelConsumptionReply::decl()]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = scanned(source, params.branch(), cancelled)?;
        let reads: Vec<_> = branch
            .countries
            .get(&params.tag)
            .into_iter()
            .flat_map(|country| &country.consumption)
            .map(FuelConsumptionRead::from)
            .collect();
        let warnings = warnings(
            reads
                .iter()
                .flat_map(|read| read.reasons.iter().flatten().cloned()),
        );
        let reply = FuelConsumptionReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            reads,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
pub struct CountryFuelDemand;
impl Product for CountryFuelDemand {
    const KEY: &'static str = "country-fuel-demand";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FuelAwardsParams;
    type Reply = FuelDemandReply;
    fn types() -> Vec<String> {
        vec![
            FuelDemandLocation::decl(),
            FuelDemandPart::decl(),
            FuelDemandCommand::decl(),
            FuelDemandFleet::decl(),
            FuelDemandAirSite::decl(),
            FuelDemandReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryFuelAwards::validate(params)
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params
            .branch_id
            .parse()
            .map_err(|error: std::num::ParseIntError| dashboard_core::invalid(error.to_string()))?;
        let read = prepared::summary_at(source, params, context, cancelled)?;
        let raw = detail::basis_reads(source, branch, params.hour, cancelled)?;
        let pack = pack_of(detail::authenticated(&raw, params)?, params.hour)
            .map_err(dashboard_core::invalid)?;
        let catalogue = std::sync::OnceLock::new();
        let mut format = |bytes: &[u8]| read_name(source, context, &catalogue, bytes);
        let mut parts = Vec::new();
        for row in occurrences(pack) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            parts.push(part_of(row, &mut format)?);
        }
        let mut commands = Vec::new();
        for group in &pack.groups {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let (recorded_name, name_reason) =
                stored_name(group.name.as_deref(), &group.name_reason, &mut format);
            commands.push(FuelDemandCommand {
                reference: group.reference.to_string(),
                parent: group.parent.to_string(),
                country: group.country.to_string(),
                army_group: group.army_group != 0,
                members: group.members.iter().map(u64::to_string).collect(),
                children: group.children.iter().map(u64::to_string).collect(),
                recorded_name,
                name_reason,
            });
        }
        let mut fleets = Vec::new();
        for fleet in &pack.fleets {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let (recorded_name, name_reason) =
                stored_name(fleet.name.as_deref(), &fleet.name_reason, &mut format);
            fleets.push(FuelDemandFleet {
                reference: fleet.reference.to_string(),
                recorded_name,
                name_reason,
            });
        }
        let mut sites = Vec::new();
        for (index, site) in pack.sites.iter().enumerate() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let mut wings = Vec::new();
            for ordinal in 0..site.wings.len() {
                wings.push(
                    serde_json::to_string(&wing_key(index, ordinal, site, &site.wings[ordinal]))
                        .map_err(|error| dashboard_core::invalid(error.to_string()))?,
                );
            }
            sites.push(FuelDemandAirSite {
                list: site.list,
                ordinal: site.ordinal,
                base: site.base.to_string(),
                container_owner: site.container_owner.map(|owner| owner.to_string()),
                capacity_raw: site.capacity_raw.map(|value| value.to_string()),
                demilitarized: site.demilitarized,
                reason: reason(&site.reason),
                wings,
            });
        }
        let topology_reason = reason(&pack.group_reason);
        let mut warnings = warnings(
            parts
                .iter()
                .flat_map(|part| [&part.reason, &part.name_reason, &part.topology_reason])
                .flatten()
                .cloned()
                .chain(sites.iter().filter_map(|site| site.reason.clone()))
                .chain(
                    commands
                        .iter()
                        .filter_map(|group| group.name_reason.clone()),
                )
                .chain(fleets.iter().filter_map(|fleet| fleet.name_reason.clone()))
                .chain(read.reasons.iter().flatten().cloned())
                .chain(topology_reason.clone()),
        );
        if let Some(Ok(catalogue)) = catalogue.get() {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(catalogue.refusal_lines());
        }
        let reply = FuelDemandReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            read,
            parts,
            commands,
            fleets,
            sites,
            topology_reason,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelDemandHistoryParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
    pub window: i32,
    pub key: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub amount_raw: Option<String>,
    pub reason: Option<String>,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
    pub segment_start_hour: i32,
    pub window: i32,
    pub key: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub runs: Vec<FuelDemandRun>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelDemandStackParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
    pub window: i32,
    pub keys: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandStackPart {
    pub key: String,
    pub runs: Vec<FuelDemandRun>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandStackReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
    pub segment_start_hour: i32,
    pub window: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub parts: Vec<FuelDemandStackPart>,
    pub warnings: Vec<String>,
}
fn unknown(first_hour: i32, last_hour: i32, reason: String) -> FuelDemandRun {
    FuelDemandRun {
        first_hour,
        last_hour,
        starts_segment: true,
        amount_raw: None,
        reason: Some(reason),
        recorded_name: None,
        name_reason: None,
    }
}
/// A tag change alone does not end the selected native actor's demand segment.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn actor_consumption<'a>(
    summary: &'a ScannedBranch,
    basis: &FuelAwardsParams,
) -> Vec<(&'a str, &'a RawConsumption)> {
    let mut reads: Vec<_> = summary
        .actor_tags(&basis.country)
        .flat_map(|tag| {
            summary.countries[tag]
                .consumption
                .iter()
                .filter(|read| read.country.to_string() == basis.country)
                .map(move |read| (tag.as_str(), read))
        })
        .collect();
    reads.sort_by_key(|(_, read)| read.first_hour);
    if reads
        .windows(2)
        .any(|reads| reads[0].1.last_hour >= reads[1].1.first_hour)
    {
        return Vec::new();
    }
    let Some(selected) = reads
        .iter()
        .position(|(tag, read)| *tag == basis.tag && read.first_hour == basis.hour)
    else {
        return Vec::new();
    };
    let joins = |left: usize, right: usize| {
        reads[left].1.last_hour.checked_add(1) == Some(reads[right].1.first_hour)
            && (reads[left].0 != reads[right].0 || !reads[right].1.starts_segment)
    };
    let mut first = selected;
    while first > 0 && joins(first - 1, first) {
        first -= 1;
    }
    let mut last = selected + 1;
    while last < reads.len() && joins(last - 1, last) {
        last += 1;
    }
    reads[first..last].to_vec()
}

/// One authenticated basis and one shared raw window for every displayed key.
fn stacked_history(
    source: &Source,
    basis: &FuelAwardsParams,
    window: i32,
    keys: &[String],
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<FuelDemandStackReply> {
    let branch = basis
        .branch_id
        .parse()
        .map_err(|error: std::num::ParseIntError| dashboard_core::invalid(error.to_string()))?;
    let summary = prepared::window_summary(source, branch, window, context, cancelled)?;
    let selected = prepared::summary_at(source, basis, context, cancelled)?;
    let basis_raw = detail::basis_reads(source, branch, basis.hour, cancelled)?;
    let pack = pack_of(detail::authenticated(&basis_raw, basis)?, basis.hour)
        .map_err(dashboard_core::invalid)?;
    let mut parsed = Vec::new();
    for key in keys {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let parsed_key = serde_json::from_str::<Key>(key)
            .map_err(|error| dashboard_core::invalid(error.to_string()))?;
        if occurrence(pack, &parsed_key).is_none() {
            return Err(dashboard_core::invalid(
                "Current fuel occurrence is absent from the original supplying read",
            ));
        }
        parsed.push(parsed_key);
    }
    let raw = detail::window_reads(source, branch, window, &summary, cancelled)?;
    let (first_hour, last_hour) = dashboard_base::products::production_lines::detail_window(window)
        .ok_or_else(|| dashboard_core::invalid("Invalid current fuel consumption window"))?;
    let catalogue = std::sync::OnceLock::new();
    let mut format = |bytes: &[u8]| read_name(source, context, &catalogue, bytes);
    let mut parts: Vec<_> = keys
        .iter()
        .map(|key| FuelDemandStackPart {
            key: key.clone(),
            runs: Vec::new(),
        })
        .collect();
    let mut next = first_hour;
    {
        let actor_reads = prepared::actor_consumption(source, basis, window, context, cancelled)?;
        for (tag, read) in actor_reads
            .into_iter()
            .filter(|(_, read)| read.last_hour >= first_hour && read.first_hour <= last_hour)
        {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if next < read.first_hour {
                for part in &mut parts {
                    part.runs.push(unknown(
                        next,
                        read.first_hour - 1,
                        "Current fuel consumption was not recorded in the selected native country/recording segment".into(),
                    ));
                }
            }
            let pack = raw
                .get(&read.first_hour)
                .and_then(|rows| rows.get(&tag))
                .ok_or_else(|| "Supplying current fuel read unavailable".to_string())
                .and_then(|row| pack_of(row, read.first_hour));
            for (part, key) in parts.iter_mut().zip(&parsed) {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let value = match &pack {
                    Ok(pack) => occurrence(pack, key)
                        .ok_or_else(|| {
                            "Selected native fuel occurrence was not present at this read"
                                .to_string()
                        })
                        .and_then(|row| {
                            part_of(row, &mut format).map_err(|error| error.to_string())
                        }),
                    Err(reason) => Err(reason.clone()),
                };
                let run = match value {
                    Ok(value) => FuelDemandRun {
                        first_hour: read.first_hour,
                        last_hour: read.last_hour.min(last_hour),
                        starts_segment: read.starts_segment,
                        amount_raw: value.amount_raw,
                        reason: value.reason,
                        recorded_name: value.recorded_name,
                        name_reason: value.name_reason,
                    },
                    Err(reason) => unknown(read.first_hour, read.last_hour.min(last_hour), reason),
                };
                part.runs.push(run);
            }
            next = read.last_hour.min(last_hour).saturating_add(1);
        }
    }
    if next <= last_hour {
        for part in &mut parts {
            part.runs.push(unknown(
                next,
                last_hour,
                "Current fuel consumption was not recorded in the selected native country/recording segment".into(),
            ));
        }
    }
    let mut warnings = warnings(
        parts
            .iter()
            .flat_map(|part| &part.runs)
            .flat_map(|run| [&run.reason, &run.name_reason])
            .flatten()
            .cloned(),
    );
    if let Some(Ok(catalogue)) = catalogue.get() {
        warnings.extend(catalogue.verification.iter().cloned());
        warnings.extend(catalogue.warnings.iter().cloned());
        warnings.extend(catalogue.refusal_lines());
    }
    Ok(FuelDemandStackReply {
        branch_id: basis.branch_id.clone(),
        tag: basis.tag.clone(),
        hour: basis.hour,
        country: basis.country.clone(),
        country_instance: basis.country_instance,
        segment_start_hour: selected.segment_start_hour,
        window,
        first_hour,
        last_hour,
        parts,
        warnings,
    })
}
pub struct CountryFuelDemandStack;
impl Product for CountryFuelDemandStack {
    const KEY: &'static str = "country-fuel-demand-stack";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FuelDemandStackParams;
    type Reply = FuelDemandStackReply;
    fn types() -> Vec<String> {
        vec![
            FuelDemandStackParams::decl(),
            FuelDemandStackPart::decl(),
            FuelDemandStackReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryFuelAwards::validate(&FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        })?;
        if dashboard_base::products::production_lines::detail_window(params.window).is_none() {
            return Err("Invalid current fuel consumption window");
        }
        let mut seen = BTreeSet::new();
        for key in &params.keys {
            let parsed = serde_json::from_str::<Key>(key)
                .map_err(|_| "Invalid current fuel occurrence key")?;
            if !seen.insert(parsed) {
                return Err("Duplicate current fuel occurrence key");
            }
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let basis = FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        };
        let reply = stacked_history(
            source,
            &basis,
            params.window,
            &params.keys,
            context,
            cancelled,
        )?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
pub struct CountryFuelDemandHistory;
impl Product for CountryFuelDemandHistory {
    const KEY: &'static str = "country-fuel-demand-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FuelDemandHistoryParams;
    type Reply = FuelDemandHistoryReply;
    fn types() -> Vec<String> {
        vec![
            FuelDemandHistoryParams::decl(),
            FuelDemandRun::decl(),
            FuelDemandHistoryReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryFuelAwards::validate(&FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        })?;
        if dashboard_base::products::production_lines::detail_window(params.window).is_none() {
            return Err("Invalid current fuel consumption window");
        }
        serde_json::from_str::<Key>(&params.key)
            .map(|_| ())
            .map_err(|_| "Invalid current fuel occurrence key")
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let basis = FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        };
        let mut stack = stacked_history(
            source,
            &basis,
            params.window,
            std::slice::from_ref(&params.key),
            context,
            cancelled,
        )?;
        let reply = FuelDemandHistoryReply {
            branch_id: stack.branch_id,
            tag: stack.tag,
            hour: stack.hour,
            country: stack.country,
            country_instance: stack.country_instance,
            segment_start_hour: stack.segment_start_hour,
            window: stack.window,
            key: params.key.clone(),
            first_hour: stack.first_hour,
            last_hour: stack.last_hour,
            runs: stack.parts.remove(0).runs,
            warnings: stack.warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
