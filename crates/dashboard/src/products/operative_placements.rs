//! Operatives working in each country, dated by the membership read.
//! Targets follow the mission country link or the operation target,
//! as selected for status text.

use super::{Product, operative_rows};
use replay::{
    frame::DecodedFrame,
    rows::{CountryLinkRow, CountryRow, OperativeMembershipRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

#[cfg(test)]
use super::LoadContext;
#[cfg(test)]
use crate::{Source, metadata::validate_campaign};
#[cfg(test)]
use replay::frame::FrameDecoder;
#[cfg(test)]
use std::sync::atomic::Ordering;

pub(crate) mod prepared;

#[cfg(test)]
mod oracle;
#[cfg(test)]
use oracle::{identities, scan};

pub struct OperativePlacements;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativePlacementParams {
    pub branch_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlacementAvailability {
    Recorded,
    FeatureAbsent,
    TablesNotRecorded,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlacementPeriod {
    pub first_hour: i32,
    pub last_hour: i32,
    pub availability: PlacementAvailability,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub count: u32,
    /// Roster/name changes split a run even when its count stays equal.
    pub operatives: Vec<PlacedOperative>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlacedOperative {
    pub id: String,
    pub name: Option<String>,
    /// A saved/localised empty name, distinct from an unreadable name.
    pub unnamed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementPair {
    /// Numeric country identities; independent of opaque tag spelling.
    pub id: String,
    pub owner: String,
    pub target: String,
    pub runs: Vec<OperativePlacementRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperativePlacementReply {
    pub branch_id: String,
    pub pairs: Vec<OperativePlacementPair>,
    /// Coverage of complete reads held within an unbroken recorded stretch.
    pub periods: Vec<PlacementPeriod>,
    /// Actual membership read hours, kept separately from compressed equal values.
    pub read_hours: Vec<i32>,
    pub warnings: Vec<String>,
}

fn recorded_operatives(frame: &DecodedFrame) -> Option<&[OperativeMembershipRow]> {
    // Snapshots keep target/name with the roster. Assignment 3 keeps its
    // mission target even with a reservation; only assignment 4 reads
    // the own operation target.
    frame.complete::<tables::OperativeMemberships>().ok()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct PairKey {
    owner: (u32, u32, String),
    target: (u32, u32, String),
}

fn resolve_rows<T: std::borrow::Borrow<CountryRow>>(
    rows: &[OperativeMembershipRow],
    countries: &HashMap<u32, T>,
    links: &[CountryLinkRow],
    localisation: Option<&installed_content::localisation::Localisation>,
    warnings: &mut std::collections::BTreeSet<String>,
) -> Result<BTreeMap<PairKey, Vec<PlacedOperative>>, String> {
    let mut result: BTreeMap<PairKey, Vec<PlacedOperative>> = BTreeMap::new();
    let mut seen = BTreeMap::new();
    let mut unnamed = crate::catalogue::Refusals::default();
    let mut refused = crate::catalogue::Refusals::default();
    for row in rows {
        if let Err(reason) =
            operative_rows::supported_kind_assignment(row.roster_kind, row.assignment)
        {
            refused.note(reason, row.id.to_string());
            continue;
        }
        if row.roster_kind != 0
            || operative_rows::captured(row.assignment)
            || !matches!(row.assignment, 3 | 4)
        {
            continue;
        }
        if row.assignment == 3 {
            match operative_rows::mission_variant(row.action_variant) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(reason) => {
                    refused.note(reason, row.id.to_string());
                    continue;
                }
            }
        } else if row
            .operation
            .require("OperativeMemberships.operation")
            .ok()
            .copied()
            .flatten()
            .is_none_or(|operation| operation == 0)
        {
            refused.note("Operation reference not recorded", row.id.to_string());
            continue;
        }
        match operative_rows::admit(&mut seen, row, links, countries) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(reason) => {
                refused.note(reason, row.id.to_string());
                continue;
            }
        }
        let key = (|| {
            let owner_id = operative_rows::identity(row.owner_country, links, countries)
                .ok_or_else(|| format!("Unresolved operative owner {}", row.owner_country))?;
            let target = row
                .target_country
                .require("OperativeMemberships.target_country")?
                .as_ref()
                .copied()
                .ok_or_else(|| "Target not recorded".to_owned())?;
            let target_id = operative_rows::identity(target, links, countries)
                .ok_or_else(|| format!("Unresolved operative target {target}"))?;
            let owner: &CountryRow = countries[&owner_id.slot].borrow();
            let target: &CountryRow = countries[&target_id.slot].borrow();
            Ok::<_, String>(PairKey {
                owner: (
                    owner_id.slot,
                    owner_id.id,
                    String::from_utf8(owner.tag.clone()).expect("validated country tag"),
                ),
                target: (
                    target_id.slot,
                    target_id.id,
                    String::from_utf8(target.tag.clone()).expect("validated country tag"),
                ),
            })
        })();
        let key = match key {
            Ok(key) => key,
            Err(reason) => {
                refused.note(reason, row.id.to_string());
                continue;
            }
        };
        let name = operative_rows::recorded_name(localisation, row);
        if name.literal_fallback {
            warnings.insert("Operative names show their saved literal/key until the recorded content stack is available.".into());
        }
        if let Some(reason) = name.missing {
            unnamed.note(
                reason,
                format!(
                    "{} ({:?})",
                    row.id,
                    String::from_utf8_lossy(
                        row.name
                            .require("OperativeMemberships.name")
                            .map(Vec::as_slice)
                            .unwrap_or_default()
                    )
                ),
            );
        }
        result.entry(key).or_default().push(PlacedOperative {
            id: row.id.to_string(),
            name: name.name,
            unnamed: name.unnamed,
        });
    }
    warnings.extend(unnamed.lines("operative names"));
    let refused = refused.lines("operative placements");
    if !refused.is_empty() {
        return Err(refused.join("\n"));
    }
    for operatives in result.values_mut() {
        operatives.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(result)
}

#[derive(Default, Serialize, Deserialize)]
struct History {
    #[serde(with = "crate::prepared_production::map_pairs")]
    pairs: BTreeMap<PairKey, OperativePlacementPair>,
    periods: Vec<PlacementPeriod>,
    reads: Vec<i32>,
    last_hour: Option<i32>,
    held: Option<Vec<(PairKey, Vec<PlacedOperative>)>>,
}

enum Observation {
    Read(BTreeMap<PairKey, Vec<PlacedOperative>>),
    Hold,
    Unavailable(PlacementAvailability),
}

impl History {
    fn project<T: std::borrow::Borrow<CountryRow>>(
        &mut self,
        countries: &HashMap<u32, T>,
    ) -> Result<(), String> {
        let Some(held) = self.held.as_ref() else {
            return Ok(());
        };
        let current = |identity: &(u32, u32, String)| {
            let row = countries
                .get(&identity.0)
                .map(|row| std::borrow::Borrow::<CountryRow>::borrow(row))
                .filter(|row| row.id == identity.1)
                .ok_or_else(|| {
                    format!(
                        "Held operative country identity {}:{} is unavailable",
                        identity.0, identity.1
                    )
                })?;
            let tag = operative_rows::tag_of(
                Some(operative_rows::CountryIdentity {
                    slot: identity.0,
                    id: identity.1,
                }),
                countries,
            )
            .ok_or_else(|| "Held operative country tag is invalid".to_owned())?;
            Ok::<_, String>((identity.0, row.id, tag))
        };
        let projected = held
            .iter()
            .map(|(key, value)| {
                Ok((
                    PairKey {
                        owner: current(&key.owner)?,
                        target: current(&key.target)?,
                    },
                    value.clone(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.held = Some(projected);
        Ok(())
    }

    fn update(&mut self, hour: i32, observation: Observation) {
        let consecutive = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        self.last_hour = Some(hour);
        if !consecutive {
            self.held = None;
        }
        let availability = match observation {
            Observation::Read(rows) => {
                self.reads.push(hour);
                self.held = Some(rows.into_iter().collect());
                PlacementAvailability::Recorded
            }
            Observation::Hold if self.held.is_some() => PlacementAvailability::Recorded,
            Observation::Hold => PlacementAvailability::TablesNotRecorded,
            Observation::Unavailable(availability) => {
                self.held = None;
                availability
            }
        };
        match self.periods.last_mut() {
            Some(period) if consecutive && period.availability == availability => {
                period.last_hour = hour
            }
            _ => self.periods.push(PlacementPeriod {
                first_hour: hour,
                last_hour: hour,
                availability,
            }),
        }
        if availability != PlacementAvailability::Recorded {
            return;
        }
        for (key, value) in self.held.as_ref().expect("recorded coverage has a read") {
            let pair = self
                .pairs
                .entry(key.clone())
                .or_insert_with(|| OperativePlacementPair {
                    id: serde_json::to_string(&(&key.owner, &key.target))
                        .expect("serializable country identities"),
                    owner: key.owner.2.clone(),
                    target: key.target.2.clone(),
                    runs: Vec::new(),
                });
            let count = u32::try_from(value.len()).expect("native operative census fits u32");
            match pair.runs.last_mut() {
                Some(run)
                    if consecutive
                        && run.last_hour.checked_add(1) == Some(hour)
                        && run.operatives == *value =>
                {
                    run.last_hour = hour
                }
                _ => pair.runs.push(OperativePlacementRun {
                    first_hour: hour,
                    last_hour: hour,
                    count,
                    operatives: value.clone(),
                }),
            }
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    history: History,
    warnings: std::collections::BTreeSet<String>,
    #[serde(skip)]
    catalogue: Option<std::sync::Arc<crate::catalogue::Catalogue>>,
    #[cfg(test)]
    #[serde(skip)]
    native_countries: operative_rows::CountryInputs,
}
impl Scan {
    fn observe_bound(
        &mut self,
        decoded: &DecodedFrame,
        inputs: &operative_rows::CountryInputs,
        bound: &Result<
            Option<std::sync::Arc<crate::catalogue::Catalogue>>,
            crate::catalogue::ContentError,
        >,
    ) -> io::Result<()> {
        let Self {
            history,
            warnings,
            catalogue,
            ..
        } = self;
        let mut observation = Observation::Unavailable(PlacementAvailability::TablesNotRecorded);
        let is_read = operative_rows::membership_read(decoded);
        if let Some(feature) = operative_rows::agency_feature(decoded) {
            if !feature {
                observation = Observation::Unavailable(PlacementAvailability::FeatureAbsent);
            } else if is_read
                && let (Some((_, links)), Some(captured)) =
                    (inputs.complete(), recorded_operatives(decoded))
            {
                if catalogue.is_none() && !captured.is_empty() {
                    *catalogue = bound
                        .as_ref()
                        .map_err(crate::catalogue::ContentError::error)?
                        .clone();
                }
                #[cfg(test)]
                super::intelligence_prepared::work(|work| work.row_visits += captured.len());
                let resolved = (inputs.unresolved.is_empty() && inputs.text)
                    .then_some(&inputs.slots)
                    .ok_or_else(|| "Unresolved operative country catalogue".to_owned())
                    .and_then(|countries| {
                        resolve_rows(
                            captured,
                            countries,
                            links,
                            catalogue
                                .as_ref()
                                .map(|catalogue| catalogue.localisation.as_ref()),
                            warnings,
                        )
                    });
                match resolved {
                    Ok(resolved) => {
                        observation = Observation::Read(resolved);
                    }
                    Err(reason) => {
                        warnings.insert(reason);
                    }
                }
            } else if !is_read && decoded.complete::<tables::OperativeMemberships>().is_ok() {
                // Complete held rows (including keyframes) retain the last
                // read's date. An unavailable/truncated table ends the hold,
                // including a scheduled failure with no measured walk.
                let projected = (|| {
                    inputs.countries.as_deref().ok_or_else(|| {
                        "Held operative country catalogue is unavailable".to_owned()
                    })?;
                    inputs
                        .links
                        .as_deref()
                        .ok_or_else(|| "Held operative country links are unavailable".to_owned())?;
                    if !inputs.unresolved.is_empty() || !inputs.text {
                        return Err("Held operative country identities are unresolved".to_owned());
                    }
                    history.project(&inputs.slots)
                })();
                match projected {
                    Ok(()) => observation = Observation::Hold,
                    Err(reason) => {
                        warnings.insert(reason);
                    }
                }
            }
        }
        history.update(decoded.hour, observation);
        Ok(())
    }
    fn reply(&self, branch: u64) -> OperativePlacementReply {
        OperativePlacementReply {
            branch_id: branch.to_string(),
            pairs: self.history.pairs.values().cloned().collect(),
            periods: self.history.periods.clone(),
            read_hours: self.history.reads.clone(),
            warnings: self.warnings.iter().cloned().collect(),
        }
    }
}

impl Product for OperativePlacements {
    const KEY: &'static str = "operative-placements";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OperativePlacementParams;
    type Reply = OperativePlacementReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid operative placement branch")
    }
    fn types() -> Vec<String> {
        vec![
            OperativePlacementParams::decl(),
            PlacementAvailability::decl(),
            PlacementPeriod::decl(),
            PlacedOperative::decl(),
            OperativePlacementRun::decl(),
            OperativePlacementPair::decl(),
            OperativePlacementReply::decl(),
        ]
    }
    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        scan(source, branch, context, cancelled)
    }
}

#[cfg(test)]
mod tests;
