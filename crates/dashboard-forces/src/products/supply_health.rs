//! Division supply-icon states, counted at their hourly reads.
//!
//! Low supply is false during naval transfer, otherwise
//! effective supply < NMilitary.LOW_SUPPLY. Stored signed operands hold the
//! received fixed5 fraction, maximum grace and remaining grace.
//! Maximum grace starts with SUPPLY_GRACE (integer hours) * 100000; the hourly
//! division update passes remaining grace and its companion to
//! the grace updater. We read stored values, never recreate
//! their update or make them fresher. DivisionStatus records these exact
//! signed i64 operands (supply_ratio, supply_grace_maximum/remaining) and
//! DivisionLandInputs the naval-transfer presence, both from the same hour's
//! authenticated division seed, keyed by full u64 handle,
//! hourly in cadence.rs.
//!
//! The icon substitutes fixed5 one when supply is not low; otherwise it uses the
//! received fraction. Strict < VERY_LOW selects frame 3, then strict < LOW selects
//! frame 2. Otherwise the supply gate selects frame 1 or hides the icon.
//! The tooltip uses the same gate for ARMY_UNIT_SUPPLY_DESC_OK/BAD and displays
//! the effective fraction. There is no separate zero-supply enum. The chart calls
//! very-low frame 3 "No supply", frames 1/2 "Strained", hidden "In supply": grace
//! can cover low received supply; frame 3 need not mean zero. Even unusual mod
//! thresholds retain the icon's comparison order.
//!
//! LOW_SUPPLY is fixed5; SUPPLY_GRACE is integer hours.
//! NGraphics.SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD and
//! SUPPLY_UNIT_COUNTER_LOW_THRESHOLD are f32 inputs. Their staged fixed5
//! conversion is shared by installed_content_core::defines::fixed5. Every value resolves
//! through the recorded content stack; province supply is no proxy.
//!
//! Owner: positive signed expeditionary_owner, else roster_owner, through
//! CountryLinks, matching DivisionCensus and Army size. Expeditionary transfer
//! preserves the sender before changing the division's host roster.
//! No player/AI filtering. Incomplete Divisions or
//! country links, unresolved owners, or any owned division without this hour's
//! status supply operands or land inputs breaks the country's whole stack.
//! Zero divisions is an exact zero denominator, not a 100% share.
//! metadata::slot_read dates every operand; equal consecutive reads compress
//! into runs without crossing gaps.
//!
//! Preparation ([`facts`]) hands each hour the countries whose holder or counts changed;
//! [`emit`] keeps them as runs on the family's series publication; [`query`] answers from it.

use dashboard_base::{content, products::Product, source::Source};
use dashboard_core::{config::ContentOptions, protocol::validate_tag};
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

pub struct SupplyHealth;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupplyHealthParams {
    pub branch_id: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    /// Exact counts; divide by their sum only for geometry/formatting.
    pub no_supply: u32,
    pub strained: u32,
    pub in_supply: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthSeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<dashboard_base::products::country_history::CountryIdentityRun>>,
    pub runs: Vec<SupplyHealthRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthReply {
    pub branch_id: String,
    pub available: bool,
    pub reason: Option<String>,
    pub missing_inputs: Vec<String>,
    pub series: Vec<SupplyHealthSeries>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summaries: Option<Vec<SupplyHealthDisplay>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthDisplay {
    pub tag: String,
    pub values: Vec<dashboard_base::prepared_chart::ChartSummary>,
}

/// One division's supply-icon operands for an hour: DivisionStatus's received
/// fraction and grace, and DivisionLandInputs's naval-transfer presence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Supply {
    pub received: i64,
    pub maximum_grace: i64,
    pub remaining_grace: i64,
    pub naval_transfer: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub(crate) struct Defines {
    pub grace: i32,
    pub low: i64,
    pub very_low_icon: i64,
    pub low_icon: i64,
}

pub fn check_content(layers: &installed_content_core::ResolvedLayers) -> io::Result<()> {
    content_defines(layers).map(|_| ())
}

fn content_defines(layers: &installed_content_core::ResolvedLayers) -> io::Result<Defines> {
    layers
        .derived::<dashboard_text::catalogue::CachedResult<Defines>>()
        .get_or_init(|| Defines::read(&installed_content_core::defines::read(layers)?))
        .map(|defines| *defines)
}

impl Defines {
    fn read(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let grace = defines
            .number("NMilitary", "SUPPLY_GRACE")
            .and_then(|_| defines.int32("NMilitary", "SUPPLY_GRACE"));
        let low = defines.fixed5("NMilitary", "LOW_SUPPLY");
        let very_low_icon = defines.fixed5("NGraphics", "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD");
        let low_icon = defines.fixed5("NGraphics", "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD");
        let errors: Vec<String> = [
            grace.as_ref().err(),
            low.as_ref().err(),
            very_low_icon.as_ref().err(),
            low_icon.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .map(ToString::to_string)
        .collect();
        if !errors.is_empty() {
            return Err(dashboard_core::invalid(format!(
                "Supply health refused {} define(s): {}",
                errors.len(),
                errors.join("; ")
            )));
        }
        Ok(Self {
            grace: grace?,
            low: low?,
            very_low_icon: very_low_icon?,
            low_icon: low_icon?,
        })
    }

    fn effective(self, row: &Supply) -> i64 {
        if row.maximum_grace <= 0 {
            return 0;
        }
        let divisor = i64::from(self.grace) * 100_000;
        let exceptional = (row.remaining_grace as u64).wrapping_add(0x7fff_ffff_ffff_fffe)
            > 0xffff_ffff_ffff_fffc;
        let grace = if divisor == 0 || exceptional {
            0xffff_ffff
        } else {
            row.remaining_grace.wrapping_mul(100_000) / divisor
        };
        row.received.max(grace).clamp(0, 100_000)
    }

    fn has_low(self, row: &Supply) -> bool {
        !row.naval_transfer && self.effective(row) < self.low
    }

    /// Native icon frame; zero means hidden. Keep all four native arms.
    fn frame(self, row: &Supply) -> usize {
        let low = self.has_low(row);
        let received = if low { row.received } else { 100_000 };
        if received < self.very_low_icon {
            3
        } else if received < self.low_icon {
            2
        } else {
            usize::from(low)
        }
    }

    /// The count a division of `row` falls in: no supply, strained, in supply.
    pub(crate) fn bucket(self, row: &Supply) -> u8 {
        match self.frame(row) {
            3 => 0,
            1 | 2 => 1,
            _ => 2,
        }
    }
}

/// What supply health is prepared under: the content's defines or why they could not be read,
/// and the content's standing warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configured {
    pub(crate) defines: Result<Defines, String>,
    pub(crate) warnings: Vec<String>,
}

impl Configured {
    /// Supply health of `source` under `options`; a refused content is named, never an error.
    pub fn read(source: &Source, options: Option<&ContentOptions>) -> io::Result<Self> {
        let mut warnings = Vec::new();
        let defines = match options
            .ok_or_else(|| dashboard_core::invalid("Game content is not configured"))
            .and_then(|options| {
                content::check_replay(
                    source.replay_ref(),
                    options.allow_unverified,
                    &mut warnings,
                )?;
                Ok(options)
            })
            .and_then(|options| source.content(&options.options))
            .and_then(|content| content_defines(&content.layers))
        {
            Ok(defines) => Ok(defines),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => Err(error.to_string()),
        };
        Ok(Self { defines, warnings })
    }

    /// Supply health under these defines and warnings.
    #[cfg(test)]
    pub(crate) fn with(defines: Result<Defines, String>, warnings: Vec<String>) -> Self {
        Self { defines, warnings }
    }

    /// One segment's projection.
    pub fn projector(
        &self,
        format: replay_data::format::FormatVersion,
    ) -> io::Result<facts::Projector> {
        facts::Projector::new(format, self.defines.as_ref().ok().copied())
    }

    /// The family's emit on a branch, from its first hour.
    pub fn emit(&self) -> emit::Emit {
        emit::Emit::new(self)
    }
}

impl Product for SupplyHealth {
    const KEY: &'static str = "supply-health";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = SupplyHealthParams;
    type Reply = SupplyHealthReply;

    fn types() -> Vec<String> {
        vec![
            SupplyHealthParams::decl(),
            SupplyHealthRun::decl(),
            SupplyHealthSeries::decl(),
            SupplyHealthReply::decl(),
            SupplyHealthDisplay::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params.tags.is_empty() {
            return Err("Choose countries for supply health");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        Ok(())
    }
}

pub mod emit;
pub mod facts;
pub mod query;
pub mod served;
#[cfg(test)]
mod tests;
