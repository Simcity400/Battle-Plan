mod breakdown;
pub mod content;
mod coverage;
#[cfg(test)]
mod tests;

use dashboard_base::{
    Source,
    products::{Country, LoadContext, Product, ReplyWarnings, breakdown::Breakdown},
};
use replay_data::rows::{EquipmentDestructionCause, EquipmentLossCoverageRow, EquipmentLossTotalRow, TableId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const AMOUNT_SCALE: i64 = 100_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LandSource {
    #[default]
    Destruction,
    CombatLog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LossCountry {
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EquipmentLossesParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    #[serde(default)]
    pub land_source: LandSource,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentLossesReply {
    pub from: i32,
    pub hour: i32,
    pub through: i32,
    pub land_source: LandSource,
    pub breakdown: Breakdown,
    pub warnings: Vec<String>,
}

impl ReplyWarnings for EquipmentLossesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossSource {
    CombatLog(u8),
    Recovered,
    SunkShips,
    SunkConvoys,
    LendLeaseSunk,
    LendLeaseSent,
    Destruction(Cause),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cause(pub EquipmentDestructionCause);

impl LossSource {
    pub fn of(code: u8) -> Option<Self> {
        match code {
            0..=3 => Some(Self::CombatLog(code)),
            4 => Some(Self::Recovered),
            EquipmentLossTotalRow::SUNK_CONVOYS => Some(Self::SunkConvoys),
            code if code == EquipmentLossTotalRow::LEND_LEASE + engine_resources::clend_lease_exchange::SUNK_KIND => Some(Self::LendLeaseSunk),
            code if code == EquipmentLossTotalRow::LEND_LEASE + engine_resources::clend_lease_exchange::DELIVERED_KIND => Some(Self::LendLeaseSent),
            code if code >= EquipmentLossTotalRow::DESTRUCTION => EquipmentDestructionCause::ALL.into_iter().find(|cause| *cause as u8 == code - EquipmentLossTotalRow::DESTRUCTION).map(|cause| Self::Destruction(Cause(cause))),
            _ => None,
        }
    }

    pub fn domain(self) -> LossDomain {
        match self {
            Self::CombatLog(_) | Self::Recovered => LossDomain::Land,
            Self::SunkShips | Self::SunkConvoys => LossDomain::Naval,
            Self::LendLeaseSunk | Self::LendLeaseSent => LossDomain::Other,
            Self::Destruction(Cause(EquipmentDestructionCause::Unknown | EquipmentDestructionCause::ScriptedStockpileFraction)) => LossDomain::Other,
            Self::Destruction(Cause(cause)) if cause.is_air() => LossDomain::Air,
            Self::Destruction(_) => LossDomain::Land,
        }
    }

    pub(super) fn mixed(self) -> bool {
        matches!(self, Self::Destruction(Cause(EquipmentDestructionCause::Unknown | EquipmentDestructionCause::ScriptedStockpileFraction)))
    }

    pub fn shown(land: LandSource) -> Vec<Self> {
        let mut sources: BTreeSet<Self> = [Self::SunkShips, Self::SunkConvoys, Self::LendLeaseSunk, Self::LendLeaseSent].into();
        let combat_log = land == LandSource::CombatLog;
        if combat_log {
            sources.extend((0..=3).map(Self::CombatLog));
            sources.insert(Self::Recovered);
        }
        sources.extend(EquipmentDestructionCause::ALL.into_iter().map(|cause| Self::Destruction(Cause(cause))).filter(|source| !combat_log || source.domain() == LossDomain::Air || source.mixed()));
        sources.into_iter().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossDomain {
    Land,
    Air,
    Naval,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LossName {
    pub key: Option<String>,
    pub template: Option<String>,
    pub text: Option<String>,
}

pub fn identity_range(slot: u32, id: u32) -> (u128, u128) {
    let low = u128::from(slot) << 73 | u128::from(id) << 41;
    (low, low | ((1u128 << 41) - 1))
}

pub(super) fn coverage_key(slot: u32, id: u32) -> u128 {
    u128::from(slot) << 32 | u128::from(id)
}

pub(super) const RUN_KEY: u128 = (EquipmentLossCoverageRow::RUN as u128) << 32 | EquipmentLossCoverageRow::RUN as u128;

pub fn leaf_of(key: u128) -> Option<(LossSource, Option<u32>)> {
    let source = LossSource::of((key >> 33) as u8)?;
    let variant = ((key >> 32) & 1 == 1).then_some(key as u32);
    Some((source, variant))
}

pub(super) fn ship_log() -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::LOGS.iter().find(|log| log.table == TableId::ShipLosses).ok_or_else(|| io::Error::other("The ship-losses log is not declared"))
}

pub(super) type Listed = (String, i32, i32, u64, u32);

pub(super) fn ships(reader: &dashboard_query::log::Reader<'_, '_>, run: u32, who: LossCountry, first: i32, last: i32) -> io::Result<Vec<Listed>> {
    use dashboard_query::log::{Cell as Logged, Pattern, facet_shift};
    let six = replay_columnar::summary::bucket_hours(0);
    let pattern = Pattern { run, first, last, mask: u128::from(u32::MAX) << facet_shift(0), value: u128::from(who.slot) << facet_shift(0), group: 0 };
    let bins = reader.bins(&[pattern], first, last, six)?;
    let ranges: Vec<(i32, i32)> = bins
        .first()
        .into_iter()
        .flatten()
        .filter(|(_, count)| *count > 0)
        .map(|&(start, _)| (start.max(i64::from(first)) as i32, (start + six - 1).min(i64::from(last)) as i32))
        .collect();
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    let number = |cell: Option<&Logged>| match cell {
        Some(Logged::Number(value)) => Some(*value),
        _ => None,
    };
    let mut out = Vec::new();
    for (_, row) in reader.rows(run, &[pattern], &ranges)? {
        if (row.key >> 80) as u32 & replay_data::rows::ShipLossRow::FIELD != who.country_id {
            continue;
        }
        let (Some(sink), Some(variant), Some(count), Some(check)) = (number(row.cells.first()), number(row.cells.get(1)), number(row.cells.get(2)), number(row.cells.get(3))) else { continue };
        let signature = row.key as u64 & replay_data::rows::ShipLossRow::SIGNATURE;
        out.push((format!("{signature:x}:{:x}", check as u64), row.hour, sink as i32, variant as u64, count as u32));
    }
    Ok(out)
}

pub struct EquipmentLosses;

impl Product for EquipmentLosses {
    const KEY: &'static str = "equipment-losses";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EquipmentLossesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: End, landSource];
    type Reply = EquipmentLossesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LandSource::decl(), EquipmentLossesParams::decl(), EquipmentLossesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid equipment losses branch")?;
        dashboard_base::products::validate_country(&params.country)?;
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        breakdown::load(source, params, context, cancelled)
    }
}
