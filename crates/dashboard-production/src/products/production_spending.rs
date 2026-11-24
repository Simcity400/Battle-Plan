
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_base::{
    catalogue::{self, Catalogue},
};
use dashboard_query::{Query, cost, identity::Holder};
use dashboard_text::variant_names::{self, Tags, Texts};
use replay_lanes::lanes::{self, Cell, History, Keys, Step};
use replay_data::rows::{DockyardRepairRateRow, OutputPurpose, ProductionCoverageRow, TableId, VariantDefinitionRow, VariantModuleRow, VariantUpgradeRow, tables};
use replay_frame::frame::{DecodedFrame, HourFrame};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(crate) mod runs;
#[cfg(test)]
mod tests;

use runs::{Amount, Context, Coverage, Piece, either, pieces, read_at, union, unknown_pieces, zip};

pub struct ProductionSpending;

pub const UNRESOLVED_MARK: &str = ":";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionSpendingParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " Also read the producer's dockyard repair rates."]
    pub repairs: bool,
    #[doc = " The window of hours asked ([`super::windows`]): running totals over its hours alone."]
    pub window: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProductionSpendingUnknown {
    NotRecorded,
    SpendingUnavailable,
    LostOutput,
    MissedPublication,
    RecordingGap,
    ObservationRestarted,
    ObservationMismatch,
    CountryChanged,
    RocketTableUnavailable,
    RocketInputInexact,
    RocketOwnerUnreadable,
    RocketDesignUnreadable,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProductionPurpose {
    New,
    Refit,
    Converted,
    Unknown,
}

impl ProductionPurpose {
    pub(crate) fn of(raw: u8) -> Self {
        match OutputPurpose::from_raw(raw) {
            Some(OutputPurpose::New) => Self::New,
            Some(OutputPurpose::Converted) => Self::Converted,
            Some(OutputPurpose::Refit) => Self::Refit,
            Some(OutputPurpose::Unknown) | None => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Domain {
    Land,
    Air,
    Naval,
}

impl Domain {
    pub fn of(category: u64) -> Self {
        if category & engine_executable::native_values::equipment_type::PLANE_MASK != 0 {
            Self::Air
        } else if category & engine_executable::native_values::equipment_type::SHIP_MASK != 0 {
            Self::Naval
        } else {
            Self::Land
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockVariantName {
    #[doc = " The read these names are from; they hold until the next entry."]
    pub first_hour: i32,
    #[doc = " The variant's display name; null when content does not name it, or when"]
    #[doc = " not known."]
    pub name: Option<String>,
    #[doc = " False from an hour the variant's names stopped being known (a gap, or a due"]
    #[doc = " read of the variant table that did not land) until the next entry."]
    pub known: bool,
    #[doc = " The design's fingerprint as read then (`equipment::design_fingerprint`: its"]
    #[doc = " name, modules and upgrade levels); null where not known. A saved variant"]
    #[doc = " choice carries it, so a later branch's other design under the same handle"]
    #[doc = " is never taken for it."]
    pub design: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingRun {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Decimal string of the fixed5 total; null is unknown, never a partial sum."]
    pub total_raw: Option<String>,
    pub reason: Option<ProductionSpendingUnknown>,
    #[doc = " The run starts another holder's or another stretch's totals: nothing is measured across it."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub break_before: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingLeaf {
    pub variant: String,
    pub type_key: String,
    pub name: Option<String>,
    pub domain: Option<Domain>,
    pub rocket: bool,
    #[doc = " What the input was for; null for Rocket output, which records none."]
    pub purpose: Option<ProductionPurpose>,
    #[doc = " For input whose purpose was not recorded: the summed magnitudes of it, dated evidence that"]
    #[doc = " grows and never cancels; empty when there never was any."]
    pub unproven: Vec<ProductionSpendingRun>,
    pub names: Vec<StockVariantName>,
    pub runs: Vec<ProductionSpendingRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionPresenceRun {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Any line or Rocket output of the producer; null cannot certify emptiness."]
    pub present: Option<bool>,
    #[doc = " The output read that found none, where present is false."]
    pub empty_census_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RepairRatePart {
    pub variant: Option<String>,
    #[doc = " The design's equipment type, or its recorded token when content cannot resolve it; empty"]
    #[doc = " for the unattributed part."]
    pub type_key: String,
    pub name: Option<String>,
    pub domain: Option<Domain>,
    pub names: Vec<StockVariantName>,
    pub runs: Vec<ProductionSpendingRun>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RepairRates {
    pub denominator: u32,
    pub total: Vec<ProductionSpendingRun>,
    #[doc = " The repaired designs' rates and the unattributed rest, which sum to the total wherever each"]
    #[doc = " is known."]
    pub parts: Vec<RepairRatePart>,
}

pub const REPAIR_DENOMINATOR: u32 = 24;

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    #[doc = " Hours either output was read, ascending."]
    pub reads: Vec<i32>,
    #[doc = " The hour each observation began: running totals count from it."]
    pub observations: Vec<i32>,
    pub equipment_observations: Vec<i32>,
    pub rocket_observations: Vec<i32>,
    pub equipment_reads: Vec<i32>,
    pub rocket_reads: Vec<i32>,
    pub rocket_key: Option<String>,
    pub rocket_name: Option<String>,
    #[doc = " One leaf per design and purpose, and per Rocket design."]
    pub leaves: Vec<ProductionSpendingLeaf>,
    #[doc = " Every purpose and Rocket output."]
    pub total: Vec<ProductionSpendingRun>,
    #[doc = " Every purpose but refits, and Rocket output."]
    pub total_without_refits: Vec<ProductionSpendingRun>,
    #[doc = " Production lines alone, every purpose."]
    pub equipment_total: Vec<ProductionSpendingRun>,
    #[doc = " Production lines alone, every purpose but refits."]
    pub equipment_total_without_refits: Vec<ProductionSpendingRun>,
    pub rocket_total: Vec<ProductionSpendingRun>,
    pub production: Vec<ProductionPresenceRun>,
    #[doc = " Asked with `repairs`: none otherwise."]
    pub repairs: Option<RepairRates>,
    pub window: u32,
    #[doc = " The hours the window holds of the timeline; first past last where it holds none."]
    pub first_hour: i32,
    pub last_hour: i32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProductionSpendingReply);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) run: u32,
    pub(crate) first: i32,
    pub(crate) last: i32,
    pub(crate) id: u32,
    pub(crate) breaks: bool,
}

pub(crate) fn spans(query: &Query<'_>, tag: &Country) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for stretch in &query.timeline.stretches {
        for run in query.holders.within(tag, stretch.first, stretch.last) {
            let Holder::Found { id, .. } = run.holder else { continue };
            let breaks = !out.is_empty();
            out.push(Span { run: stretch.run, first: run.first, last: run.last, id, breaks });
        }
    }
    out
}

pub(crate) struct Read {
    pub(crate) history: History,
    at: Vec<usize>,
}

impl Read {
    pub(crate) fn of(query: &Query<'_>, span: &Span, table: TableId, names: &[&str], keys: Keys<'_>) -> io::Result<Self> {
        let columns = names.iter().map(|name| dashboard_declare::field(table, name)).collect::<io::Result<Vec<_>>>()?;
        let history = cost::storage(|| lanes::history(query.replay.recording(), span.run, table, &columns, keys, span.first, span.last))?;
        let at = columns
            .iter()
            .map(|column| history.columns.binary_search(column).map_err(|_| invalid(format!("{table:?} history lacks a column it was asked"))))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { history, at })
    }

    pub(crate) fn steps(&self, key: u128) -> &[Step] {
        self.history.keys.binary_search_by_key(&key, |(held, _)| *held).map_or(&[], |at| &self.history.keys[at].1)
    }

    pub(crate) fn number(&self, cells: &[Cell], column: usize) -> Option<i64> {
        match cells.get(self.at[column]) {
            Some(Cell::Number(value)) => Some(*value),
            _ => None,
        }
    }

    fn pieces<T: Clone + PartialEq>(&self, key: u128, span: &Span, mut value: impl FnMut(&Self, &[Cell]) -> T) -> Vec<Piece<Option<T>>> {
        pieces(self.steps(key).iter().map(|step| (step.hour, step.cells.as_deref().map(|cells| value(self, cells)))), span.first, span.last, None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) spent: Option<i64>,
    pub(crate) reason: u8,
    pub(crate) unproven: Option<i64>,
}

pub(crate) type LeafKey = (bool, u64, Option<u8>);

pub(crate) struct SpanRead {
    pub(crate) span: Span,
    lines: Vec<Piece<Context>>,
    rocket: Vec<Piece<Context>>,
    country: Vec<Piece<Option<(Option<i64>, Option<i64>)>>>,
    pub(crate) rows: BTreeMap<LeafKey, Vec<Piece<Option<Row>>>>,
    pub(crate) archetypes: BTreeMap<u64, u32>,
    equipment_reads: Vec<i32>,
    rocket_reads: Vec<i32>,
    pub(crate) equipment_observations: BTreeSet<i32>,
    rocket_observations: BTreeSet<i32>,
    pub(crate) repairs: Option<RepairRead>,
}

pub(crate) struct RepairRead {
    total: Vec<Piece<Option<(Option<i64>, bool)>>>,
    pub(crate) parts: BTreeMap<Option<u64>, Vec<Piece<Option<Option<i64>>>>>,
}

const RUN: u128 = ProductionCoverageRow::RUN as u128;

pub(crate) fn reads_of(query: &Query<'_>, span: &Span, table: TableId) -> io::Result<Vec<i32>> {
    let runs = cost::storage(|| lanes::reads(query.replay.recording(), span.run, table, span.first, span.last))?;
    Ok(runs.into_iter().flat_map(|(first, last)| first..=last).collect())
}

impl SpanRead {
    pub(crate) fn read(query: &Query<'_>, span: Span, repairs: bool) -> io::Result<Self> {
        let id = u128::from(span.id);
        let country = Read::of(query, &span, TableId::ProductionCountryTotals, &["spent", "spent_without_refits"], Keys::These(&[id]))?;
        let country = country.pieces(id, &span, |read, cells| (read.number(cells, 0), read.number(cells, 1)));

        let coverage_keys = [id << 8 | u128::from(ProductionCoverageRow::ROCKET), RUN << 8 | u128::from(ProductionCoverageRow::LINES), RUN << 8 | u128::from(ProductionCoverageRow::ROCKET)];
        let coverage = Read::of(query, &span, TableId::ProductionCoverage, &["observation_since", "spending_unknown_from", "spending_reason"], Keys::These(&coverage_keys))?;
        let cells = |read: &Read, cells: &[Cell]| Coverage {
            observation: read.number(cells, 0).and_then(|hour| i32::try_from(hour).ok()),
            unknown_from: read.number(cells, 1).and_then(|hour| i32::try_from(hour).ok()),
            reason: read.number(cells, 2).map_or(0, |code| code as u8),
        };
        let [own_rocket, run_lines, run_rocket] = coverage_keys.map(|key| coverage.pieces(key, &span, cells));
        let observations = |rows: &[Piece<Option<Coverage>>]| rows.iter().filter_map(|piece| piece.2.and_then(|row| row.observation)).collect::<BTreeSet<_>>();
        let equipment_observations = observations(&run_lines);
        let rocket_observations = observations(&run_rocket);

        let shown: Vec<Piece<bool>> = country.iter().map(|&(first, last, row)| (first, last, row.is_some())).collect();
        let lines = zip(&shown, &unknown_pieces(&run_lines), |&shown, why| Context::of(shown, *why));

        let totals = Read::of(query, &span, TableId::ProductionTotals, &["spent", "spent_reason", "unproven", "archetype"], Keys::Range(id << 72, id << 72 | ((1u128 << 72) - 1)))?;
        let mut rows = BTreeMap::new();
        let mut archetypes = BTreeMap::new();
        for (key, steps) in &totals.history.keys {
            let variant = (key >> 8) as u64;
            if let Some(archetype) = steps.iter().rev().filter_map(|step| step.cells.as_deref()).filter_map(|cells| totals.number(cells, 3)).find(|&token| token > 0) {
                archetypes.insert(variant, archetype as u32);
            }
            let row = totals.pieces(*key, &span, |read, cells| Row { spent: read.number(cells, 0), reason: read.number(cells, 1).map_or(0, |code| code as u8), unproven: read.number(cells, 2) });
            rows.insert((false, variant, Some(*key as u8)), row);
        }

        let rockets = Read::of(query, &span, TableId::RocketProductionOutput, &["spent_raw"], Keys::Ranges(&[(0, 0), (id << 64 | 1, id << 64 | u128::from(u64::MAX))]))?;
        let world: Vec<Piece<bool>> = rockets.pieces(0, &span, |_, _| ()).into_iter().map(|(first, last, row)| (first, last, row.is_some())).collect();
        let rocket_unknown = zip(&unknown_pieces(&run_rocket), &unknown_pieces(&own_rocket), either);
        let rocket = zip(&world, &rocket_unknown, |&shown, why| if shown { Context::of(true, *why) } else { Context::Unknown(ProductionSpendingUnknown::RocketTableUnavailable) });
        for (key, _) in rockets.history.keys.iter().filter(|(key, _)| *key != 0) {
            let row = rockets.pieces(*key, &span, |read, cells| Row { spent: read.number(cells, 0), reason: 0, unproven: None });
            rows.insert((true, *key as u64, None), row);
        }

        let repairs = repairs.then(|| RepairRead::read(query, &span)).transpose()?;
        Ok(Self {
            lines,
            rocket,
            country,
            rows,
            archetypes,
            equipment_reads: reads_of(query, &span, TableId::ProductionLineOutput)?,
            rocket_reads: reads_of(query, &span, TableId::RocketProductionOutput)?,
            equipment_observations,
            rocket_observations,
            repairs,
            span,
        })
    }

    fn context(&self, rocket: bool) -> &[Piece<Context>] {
        if rocket { &self.rocket } else { &self.lines }
    }

    pub(crate) fn amounts(&self, key: &LeafKey, cell: impl Fn(&Row) -> Option<i64>) -> Vec<Piece<Amount>> {
        let absent = [(self.span.first, self.span.last, None)];
        let rows = self.rows.get(key).map_or(&absent[..], Vec::as_slice);
        let reason = ProductionSpendingUnknown::SpendingUnavailable;
        zip(self.context(key.0), rows, |context, row| match row {
            Some(row) => context.amount(Some(cell(row)), if key.0 { reason } else { runs::reason(row.reason) }),
            None => context.amount(None, reason),
        })
    }

    pub(crate) fn equipment(&self, refits: bool) -> Vec<Piece<Amount>> {
        zip(&self.lines, &self.country, |context, row| context.amount(row.map(|row| if refits { row.0 } else { row.1 }), ProductionSpendingUnknown::SpendingUnavailable))
    }

    fn rocket_total(&self) -> Vec<Piece<Amount>> {
        let mut total: Vec<Piece<Amount>> = self.rocket.iter().map(|&(first, last, context)| (first, last, context.amount(None, ProductionSpendingUnknown::SpendingUnavailable))).collect();
        for key in self.rows.keys().filter(|key| key.0) {
            total = zip(&total, &self.amounts(key, |row| row.spent), |a, b| a.plus(*b));
        }
        total
    }

    fn presence(&self) -> Vec<ProductionPresenceRun> {
        let held: Vec<(i32, i32)> = self.rows.values().flatten().filter(|piece| piece.2.is_some()).map(|piece| (piece.0, piece.1)).collect();
        let present = union(held, self.span.first, self.span.last);
        zip(&self.country, &present, |row, &present| row.map(|_| present))
            .into_iter()
            .map(|(first_hour, last_hour, present)| ProductionPresenceRun {
                first_hour,
                last_hour,
                present,
                empty_census_hour: (present == Some(false)).then(|| read_at(&self.equipment_reads, first_hour)).flatten(),
            })
            .collect()
    }
}

impl RepairRead {
    fn read(query: &Query<'_>, span: &Span) -> io::Result<Self> {
        let owner = u128::from(span.id) << 72;
        let rates = Read::of(query, span, TableId::DockyardRepairRates, &["rate", "split_known"], Keys::Range(owner, owner | ((1u128 << 72) - 1)))?;
        let total_key = owner | u128::from(DockyardRepairRateRow::TOTAL) << 64;
        let total = rates.pieces(total_key, span, |read, cells| (read.number(cells, 0), read.number(cells, 1) == Some(1)));
        let mut parts = BTreeMap::new();
        for (key, _) in &rates.history.keys {
            let part = (key >> 64) as u8;
            let design = match part {
                DockyardRepairRateRow::DESIGN => Some(*key as u64),
                DockyardRepairRateRow::UNATTRIBUTED => None,
                _ => continue,
            };
            parts.insert(design, rates.pieces(*key, span, |read, cells| read.number(cells, 0)));
        }
        parts.entry(None).or_insert_with(|| vec![(span.first, span.last, None)]);
        Ok(Self { total, parts })
    }

    fn span(&self) -> [Piece<()>; 1] {
        [(self.total.first().map_or(0, |piece| piece.0), self.total.last().map_or(0, |piece| piece.1), ())]
    }

    pub(crate) fn total(&self) -> Vec<Piece<Amount>> {
        zip(&self.total, &self.span(), |row, _| match row {
            None => Amount::Unknown(ProductionSpendingUnknown::NotRecorded),
            Some((Some(rate), _)) => Amount::Known(i128::from(*rate)),
            Some((None, _)) => Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable),
        })
    }

    fn part(&self, design: Option<u64>) -> Vec<Piece<Amount>> {
        let [(first, last, ())] = self.span();
        let absent = [(first, last, None)];
        let own = self.parts.get(&design).map_or(&absent[..], Vec::as_slice);
        zip(&self.total, own, |total, own| match (total, own) {
            (None, _) => Amount::Unknown(ProductionSpendingUnknown::NotRecorded),
            (Some((None, _)), _) => Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable),
            (Some((Some(_), false)), _) | (_, Some(None)) => Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable),
            (_, None) => Amount::Known(0),
            (_, Some(Some(rate))) => Amount::Known(i128::from(*rate)),
        })
    }
}

fn append(runs: &mut Vec<ProductionSpendingRun>, pieces: &[Piece<Amount>], breaks: bool) {
    for (index, &(first_hour, last_hour, amount)) in pieces.iter().enumerate() {
        let (total_raw, reason) = match amount {
            Amount::Known(value) => (Some(value.to_string()), None),
            Amount::Unknown(reason) => (None, Some(reason)),
        };
        let break_before = (index == 0 && breaks).then_some(true);
        match runs.last_mut() {
            Some(open) if break_before.is_none() && open.last_hour.checked_add(1) == Some(first_hour) && open.total_raw == total_raw && open.reason == reason => open.last_hour = last_hour,
            _ => runs.push(ProductionSpendingRun { first_hour, last_hour, total_raw, reason, break_before }),
        }
    }
}

fn runs_of(spans: &[SpanRead], mut each: impl FnMut(&SpanRead) -> Vec<Piece<Amount>>) -> Vec<ProductionSpendingRun> {
    let mut runs = Vec::new();
    for span in spans {
        append(&mut runs, &each(span), span.span.breaks);
    }
    runs
}

#[derive(Clone, Debug)]
pub(crate) struct Design {
    pub(crate) type_key: String,
    pub(crate) name: Option<String>,
    pub(crate) domain: Option<Domain>,
    pub(crate) names: Vec<StockVariantName>,
}

pub(crate) struct Content {
    pub(crate) catalogue: Option<Arc<Catalogue>>,
    pub(crate) units: Option<Arc<content_reader_key_names::equipment::Database>>,
}

impl Content {
    pub(crate) fn read(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> Self {
        let catalogue = match context.content().and_then(|options| catalogue::catalogue(source, options)) {
            Ok(catalogue) => {
                warnings.extend(catalogue.refusal_lines());
                Some(catalogue)
            }
            Err(error) => {
                warnings.push(format!("Design and equipment type names are unavailable: {error}"));
                None
            }
        };
        let units = catalogue.as_ref().and_then(|catalogue| match catalogue::equipment_units(catalogue) {
            Ok(units) => {
                warnings.extend(units.misreads.iter().cloned());
                Some(units)
            }
            Err(error) => {
                warnings.push(format!("Equipment types are unavailable: {error}"));
                None
            }
        });
        Self { catalogue, units }
    }

    pub(crate) fn archetype(&self, spelling: &str) -> Option<(String, Option<String>, Domain)> {
        let units = self.units.as_ref()?;
        let ordinal = units.get(spelling)?;
        let at = units.equipment[ordinal].archetype.unwrap_or(ordinal);
        let equipment = &units.equipment[at];
        let key = equipment.name_str().ok()?.to_owned();
        if key.starts_with(UNRESOLVED_MARK) {
            return None;
        }
        let name = self.catalogue.as_ref().and_then(|catalogue| {
            catalogue.localisation.get(&key)?;
            catalogue.localised(&key, catalogue::View::Plain)
        });
        Some((key, name, Domain::of(units.category(equipment))))
    }
}

pub(crate) fn designs(query: &Query<'_>, span: &Span, variants: &BTreeMap<u64, u32>, content: &Content, notes: &mut BTreeSet<String>) -> io::Result<HashMap<u64, Design>> {
    if variants.is_empty() {
        return Ok(HashMap::new());
    }
    let recording = query.replay.recording();
    let keys: Vec<u128> = variants.keys().map(|&variant| u128::from(variant)).collect();
    let parts: Vec<(u128, u128)> = keys.iter().map(|&variant| (variant << 32, variant << 32 | u128::from(u32::MAX))).collect();
    let mut frame: DecodedFrame = cost::storage(|| {
        replay_lanes::at::frame(
            recording,
            span.run,
            span.last,
            &[
                replay_lanes::at::Read { table: TableId::VariantDefinitions, columns: None, keys: Keys::These(&keys) },
                replay_lanes::at::Read { table: TableId::VariantModules, columns: None, keys: Keys::Ranges(&parts) },
                replay_lanes::at::Read { table: TableId::VariantUpgrades, columns: None, keys: Keys::Ranges(&parts) },
            ],
        )
    })?;
    let definitions: HashMap<u64, VariantDefinitionRow> = frame
        .complete::<tables::VariantDefinitions>()
        .map(|rows| rows.iter().map(|row| (row.id, row.clone())).collect())
        .unwrap_or_default();
    let since: HashMap<u64, i32> = cost::storage(|| lanes::history(recording, span.run, TableId::VariantDefinitions, &[0], Keys::These(&keys), span.first, span.last))?
        .keys
        .iter()
        .filter_map(|(key, steps)| steps.iter().find(|step| step.cells.is_some()).map(|step| (*key as u64, step.hour.max(span.first))))
        .collect();
    let modules: Option<Vec<VariantModuleRow>> = frame.complete::<tables::VariantModules>().ok().map(|rows| rows.to_vec());
    let upgrades: Option<Vec<VariantUpgradeRow>> = frame.complete::<tables::VariantUpgrades>().ok().map(|rows| rows.to_vec());
    let mut tokens: BTreeSet<u128> = BTreeSet::new();
    for (variant, archetype) in variants {
        tokens.extend((*archetype > 0).then_some(u128::from(*archetype)));
        if let Some(definition) = definitions.get(variant) {
            tokens.extend([u128::from(definition.equipment_token), u128::from(definition.archetype_token)].into_iter().filter(|&token| token > 0));
        }
    }
    for row in modules.iter().flatten() {
        tokens.extend([u128::from(row.slot), u128::from(row.definition_token)].into_iter().filter(|&token| token > 0));
    }
    tokens.extend(upgrades.iter().flatten().filter_map(|row| u32::try_from(row.definition_token).ok()).filter(|&token| token > 0).map(u128::from));
    let tokens: Vec<u128> = tokens.into_iter().collect();
    let named = cost::storage(|| {
        replay_lanes::at::frame(
            recording,
            span.run,
            span.last,
            &[
                replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: Keys::These(&tokens) },
                replay_lanes::at::Read { table: TableId::Countries, columns: None, keys: Keys::All },
                replay_lanes::at::Read { table: TableId::CountryLinks, columns: None, keys: Keys::All },
                replay_lanes::at::Read { table: TableId::CountryAppearance, columns: None, keys: Keys::All },
            ],
        )
    })?;
    for table in [TableId::LexicalTokens, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance] {
        frame.tables[table.index()] = named.tables[table.index()].clone();
    }
    let lexical = frame.complete::<tables::LexicalTokens>().unwrap_or(&[]);
    let spell = |token: u32| -> Option<String> {
        let at = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
        std::str::from_utf8(content_reader_parser::parser::c_string(&lexical[at].literal)).ok().filter(|text| !text.is_empty()).map(str::to_owned)
    };
    let known_spelling = |token: u32| -> Result<String, String> {
        if token == 0 {
            return Ok(String::new());
        }
        let at = lexical.binary_search_by_key(&token, |row| row.token).map_err(|_| format!("lexical tokens lacks equipment token {token}"))?;
        variant_names::text(&lexical[at].literal, &format!("equipment lexer token {token}"))
    };
    let tags = Tags::from_tables(HourFrame::from(&frame), true).or_else(|_| Tags::from_tables(HourFrame::from(&frame), false));
    if let Err(why) = &tags {
        notes.insert(format!("Countries' tags were not read for design names: {why}"));
    }
    let empty = Tags::new(&[], &[], None);
    let texts = content.catalogue.as_deref().map(|catalogue| catalogue as &dyn Texts);
    let mut out = HashMap::new();
    for (&variant, &archetype) in variants {
        let definition = definitions.get(&variant);
        let archetype = Some(archetype).filter(|&token| token > 0).or_else(|| definition.map(|definition| definition.archetype_token).filter(|&token| token > 0));
        let spelling = archetype.and_then(spell);
        let resolved = spelling.as_deref().and_then(|spelling| content.archetype(spelling));
        let (type_key, name, domain) = match (resolved, &spelling) {
            (Some((key, name, domain)), _) => (key, name, Some(domain)),
            (None, Some(spelling)) => {
                if content.units.is_some() {
                    notes.insert(format!("Equipment type {spelling} is not in the recorded content; its designs stay apart."));
                }
                (format!("{UNRESOLVED_MARK}{spelling}"), None, None)
            }
            (None, None) => {
                notes.insert("A design's equipment type was not recorded; it stays apart.".to_owned());
                (format!("{UNRESOLVED_MARK}variant/{variant}"), None, None)
            }
        };
        let names = match definition {
            Some(definition) => {
                let mut raw = match &tags {
                    Ok(tags) => variant_names::raw_variant(definition, span.last, tags, lexical),
                    Err(_) => variant_names::raw_variant(definition, span.last, &empty, lexical),
                };
                let own = of_variant(modules.as_deref(), variant, |row: &VariantModuleRow| row.variant);
                let upgraded = of_variant(upgrades.as_deref(), variant, |row: &VariantUpgradeRow| row.variant);
                if let (Some(own), Some(upgraded)) = (own, upgraded) {
                    let own: Vec<&VariantModuleRow> = own.iter().collect();
                    let upgraded: Vec<&VariantUpgradeRow> = upgraded.iter().collect();
                    variant_names::with_design(&mut raw, &own, &upgraded, &known_spelling);
                }
                let (name, _) = variant_names::variant_names(&raw, texts);
                StockVariantName { first_hour: since.get(&variant).copied().unwrap_or(span.last), name, known: true, design: raw.design }
            }
            None => StockVariantName { first_hour: span.first, name: None, known: false, design: None },
        };
        out.insert(variant, Design { type_key, name, domain, names: vec![names] });
    }
    Ok(out)
}

fn of_variant<T>(rows: Option<&[T]>, variant: u64, key: impl Fn(&T) -> u64) -> Option<&[T]> {
    rows.map(|rows| &rows[rows.partition_point(|row| key(row) < variant)..rows.partition_point(|row| key(row) <= variant)])
}

fn leaves(spans: &[SpanRead], named: &[HashMap<u64, Design>]) -> Vec<ProductionSpendingLeaf> {
    let keys: BTreeSet<LeafKey> = spans.iter().flat_map(|span| span.rows.keys().copied()).collect();
    keys.into_iter()
        .map(|key| {
            let (rocket, variant, purpose) = key;
            let mut design: Option<Design> = None;
            let mut names: Vec<StockVariantName> = Vec::new();
            for (span, named) in spans.iter().zip(named) {
                if !span.rows.contains_key(&key) {
                    continue;
                }
                if let Some(found) = named.get(&variant) {
                    for name in &found.names {
                        if names.last().is_none_or(|last| (&last.name, last.known) != (&name.name, name.known)) {
                            names.push(name.clone());
                        }
                    }
                    design = Some(found.clone());
                }
            }
            let design = design.unwrap_or_else(|| Design { type_key: format!("{UNRESOLVED_MARK}variant/{variant}"), name: None, domain: None, names: Vec::new() });
            let unknown_purpose = purpose == Some(OutputPurpose::Unknown.raw());
            let unproven = if unknown_purpose { runs_of(spans, |span| span.amounts(&key, |row| row.unproven)) } else { Vec::new() };
            let evidenced = unproven.iter().any(|run| run.total_raw.as_deref().is_some_and(|raw| raw != "0"));
            ProductionSpendingLeaf {
                variant: variant.to_string(),
                type_key: design.type_key,
                name: design.name,
                domain: design.domain,
                rocket,
                purpose: purpose.map(ProductionPurpose::of),
                unproven: if evidenced { unproven } else { Vec::new() },
                names,
                runs: runs_of(spans, |span| span.amounts(&key, |row| row.spent)),
            }
        })
        .collect()
}

fn repairs(spans: &[SpanRead], named: &[HashMap<u64, Design>]) -> RepairRates {
    let total = runs_of(spans, |span| span.repairs.as_ref().map_or_else(|| vec![(span.span.first, span.span.last, Amount::Unknown(ProductionSpendingUnknown::NotRecorded))], RepairRead::total));
    let designs: BTreeSet<Option<u64>> = spans.iter().filter_map(|span| span.repairs.as_ref()).flat_map(|repairs| repairs.parts.keys().copied()).collect();
    let parts = designs
        .into_iter()
        .map(|design| {
            let runs = runs_of(spans, |span| span.repairs.as_ref().map_or_else(|| vec![(span.span.first, span.span.last, Amount::Unknown(ProductionSpendingUnknown::NotRecorded))], |repairs| repairs.part(design)));
            let Some(variant) = design else {
                return RepairRatePart { variant: None, type_key: String::new(), name: None, domain: None, names: Vec::new(), runs };
            };
            let found = named.iter().rev().find_map(|named| named.get(&variant));
            let mut names: Vec<StockVariantName> = Vec::new();
            for named in named {
                for name in named.get(&variant).map_or(&[][..], |found| found.names.as_slice()) {
                    if names.last().is_none_or(|last| (&last.name, last.known) != (&name.name, name.known)) {
                        names.push(name.clone());
                    }
                }
            }
            RepairRatePart {
                variant: Some(variant.to_string()),
                type_key: found.map_or_else(|| format!("{UNRESOLVED_MARK}variant/{variant}"), |found| found.type_key.clone()),
                name: found.and_then(|found| found.name.clone()),
                domain: found.and_then(|found| found.domain),
                names,
                runs,
            }
        })
        .collect();
    RepairRates { denominator: REPAIR_DENOMINATOR, total, parts }
}

type RocketName = Result<(String, Option<String>), String>;

fn rocket_name(buildings: &content_reader_key_names::buildings::Buildings, texts: Option<&dyn Texts>) -> RocketName {
    let keys: Vec<&String> = buildings
        .declaration_order
        .iter()
        .filter(|key| buildings.factory_flags.get(*key).is_some_and(|flags| flags.rocket()) || buildings.ambiguous_factory_types.contains(*key))
        .collect();
    let [key] = keys.as_slice() else {
        return Err(format!("{} content building definitions can classify Rocket production; the source name is not known", keys.len()));
    };
    if buildings.ambiguous_factory_types.contains(*key) || buildings.repeated_types.contains_key(*key) {
        return Err(format!("{key}: repeated Rocket building definition is ambiguous"));
    }
    Ok(((*key).clone(), texts.and_then(|texts| texts.text(key))))
}

#[derive(Default)]
struct RocketNameCache(Option<(Arc<Catalogue>, RocketName)>);

fn cached_rocket_name(source: &Source, catalogue: &Arc<Catalogue>) -> RocketName {
    let state = source.state::<RocketNameCache>();
    let mut held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((from, name)) = &held.0
        && Arc::ptr_eq(from, catalogue)
    {
        return name.clone();
    }
    let name = content_reader_key_names::buildings::read(&catalogue.content.layers).map_err(|error| error.to_string()).and_then(|buildings| rocket_name(&buildings, Some(&**catalogue as &dyn Texts)));
    held.0 = Some((Arc::clone(catalogue), name.clone()));
    name
}

fn sorted(hours: impl IntoIterator<Item = i32>) -> Vec<i32> {
    hours.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

fn spans_within(query: &Query<'_>, tag: &Country, (first, last): (i32, i32)) -> Vec<Span> {
    spans(query, tag)
        .into_iter()
        .filter_map(|span| {
            let (from, to) = (span.first.max(first), span.last.min(last));
            (from <= to).then_some(Span { first: from, last: to, breaks: span.breaks && from == span.first, ..span })
        })
        .collect()
}

fn reply(source: &Source, query: &Query<'_>, params: &ProductionSpendingParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ProductionSpendingReply> {
    let (first_hour, last_hour) = dashboard_world::products::windows::within(query, params.window)?;
    let mut read = Vec::new();
    for span in spans_within(query, &params.country, (first_hour, last_hour)) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        read.push(SpanRead::read(query, span, params.repairs)?);
    }
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let mut notes = BTreeSet::new();
    let mut named = Vec::with_capacity(read.len());
    for span in &read {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut variants: BTreeMap<u64, u32> = span.rows.keys().map(|&(_, variant, _)| (variant, span.archetypes.get(&variant).copied().unwrap_or(0))).collect();
        for design in span.repairs.iter().flat_map(|repairs| repairs.parts.keys().flatten()) {
            variants.entry(*design).or_insert(0);
        }
        named.push(designs(query, &span.span, &variants, &content, &mut notes)?);
    }
    warnings.extend(notes);
    let equipment = runs_of(&read, |span| span.equipment(true));
    let equipment_without = runs_of(&read, |span| span.equipment(false));
    let rocket = runs_of(&read, SpanRead::rocket_total);
    let total = runs_of(&read, |span| zip(&span.equipment(true), &span.rocket_total(), |a, b| a.plus(*b)));
    let total_without = runs_of(&read, |span| zip(&span.equipment(false), &span.rocket_total(), |a, b| a.plus(*b)));
    let equipment_reads = sorted(read.iter().flat_map(|span| span.equipment_reads.iter().copied()));
    let rocket_reads = sorted(read.iter().flat_map(|span| span.rocket_reads.iter().copied()));
    let equipment_observations = sorted(read.iter().flat_map(|span| span.equipment_observations.iter().copied()));
    let rocket_observations = sorted(read.iter().flat_map(|span| span.rocket_observations.iter().copied()));
    let leaves = leaves(&read, &named);
    let (mut rocket_key, mut rocket_name) = (None, None);
    if let Some(catalogue) = content.catalogue.as_ref().filter(|_| leaves.iter().any(|leaf| leaf.rocket)) {
        match cached_rocket_name(source, catalogue) {
            Ok((key, name)) => (rocket_key, rocket_name) = (Some(key), name),
            Err(reason) => warnings.push(format!("Rocket production source name is unavailable: {reason}.")),
        }
    }
    let reply = ProductionSpendingReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.nearest_name(&params.country, query.timeline.last),
        decimals: 5,
        reads: sorted(equipment_reads.iter().chain(&rocket_reads).copied()),
        observations: sorted(equipment_observations.iter().chain(&rocket_observations).copied()),
        equipment_observations,
        rocket_observations,
        equipment_reads,
        rocket_reads,
        rocket_key,
        rocket_name,
        leaves,
        total,
        total_without_refits: total_without,
        equipment_total: equipment,
        equipment_total_without_refits: equipment_without,
        rocket_total: rocket,
        production: read.iter().flat_map(SpanRead::presence).collect(),
        repairs: params.repairs.then(|| repairs(&read, &named)),
        window: params.window,
        first_hour,
        last_hour,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

impl Product for ProductionSpending {
    const KEY: &'static str = "production-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionSpendingParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, repairs, window];
    type Reply = ProductionSpendingReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_world::products::windows::hours(params.window).ok_or("Invalid production spending window")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ProductionSpendingParams::decl(),
            ProductionSpendingReply::decl(),
            ProductionSpendingUnknown::decl(),
            ProductionPurpose::decl(),
            Domain::decl(),
            StockVariantName::decl(),
            ProductionSpendingRun::decl(),
            ProductionSpendingLeaf::decl(),
            ProductionPresenceRun::decl(),
            RepairRatePart::decl(),
            RepairRates::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        reply(source, &query, params, context, cancelled)
    }
}
