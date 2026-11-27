
use dashboard_base::products::{Country, LoadContext, Product, at_hour, breakdown::Breakdown, validate_country};
use dashboard_base::Source;
use dashboard_base::catalogue::{self, Catalogue};
use dashboard_query::{Query, cost, identity::Holder};
use dashboard_text::variant_names::{self, Tags, Texts};
use replay_lanes::lanes::{self, Cell, History, Keys, Step};
use replay_data::rows::{DockyardRepairRateRow, LexicalTokenRow, OutputPurpose, ProductionCoverageRow, TableId, VariantDefinitionRow, VariantModuleRow, VariantUpgradeRow, tables};
use replay_frame::frame::{DecodedFrame, HourFrame};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

mod breakdown;
pub(crate) mod runs;
#[cfg(test)]
mod tests;

use runs::{Amount, Context, Coverage, Piece, either, pieces, unknown_pieces, zip};

pub struct ProductionSpending;

pub const UNRESOLVED_MARK: &str = ":";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProductionSpendingScope {
    Military,
    Dockyards,
    DockyardPurposes,
    AirPurposes,
    LandPurposes,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionSpendingParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    pub scope: ProductionSpendingScope,
    #[serde(default)]
    pub include_refits: bool,
    #[doc = " The first hour of a chosen range: spending from it through `hour`; from the observation start when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
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

pub const REPAIR_DENOMINATOR: u32 = 24;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingReply {
    #[doc = " The branch's first hour: spending is counted from it."]
    pub from: i32,
    pub hour: i32,
    #[doc = " Everything in the reply stands unchanged from `hour` through this hour."]
    pub through: i32,
    pub breakdown: Breakdown,
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
    pub(crate) archetype_since: BTreeMap<u64, i32>,
    pub(crate) equipment_observations: BTreeSet<i32>,
    pub(crate) rocket_observations: BTreeSet<i32>,
    pub(crate) repairs: Option<RepairRead>,
}

pub(crate) struct RepairRead {
    pub(super) total: Vec<Piece<Option<(Option<i64>, bool)>>>,
    pub(crate) parts: BTreeMap<Option<u64>, Vec<Piece<Option<Option<i64>>>>>,
    pub(super) index: breakdown::RepairIndex,
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
        let mut rocket_observations = observations(&run_rocket);
        rocket_observations.extend(observations(&own_rocket));

        let shown: Vec<Piece<bool>> = country.iter().map(|&(first, last, row)| (first, last, row.is_some())).collect();
        let lines = zip(&shown, &unknown_pieces(&run_lines), |&shown, why| Context::of(shown, *why));

        let totals = Read::of(query, &span, TableId::ProductionTotals, &["spent", "spent_reason", "unproven", "archetype"], Keys::Range(id << 72, id << 72 | ((1u128 << 72) - 1)))?;
        let mut rows = BTreeMap::new();
        let mut archetypes = BTreeMap::new();
        let mut archetype_since: BTreeMap<u64, i32> = BTreeMap::new();
        for (key, steps) in &totals.history.keys {
            let variant = (key >> 8) as u64;
            if let Some(archetype) = steps.iter().rev().filter_map(|step| step.cells.as_deref()).filter_map(|cells| totals.number(cells, 3)).find(|&token| token > 0) {
                archetypes.insert(variant, archetype as u32);
            }
            if let Some(first) = steps.iter().find(|step| step.cells.as_deref().and_then(|cells| totals.number(cells, 3)).is_some_and(|token| token > 0)) {
                let first = first.hour.max(span.first);
                archetype_since.entry(variant).and_modify(|held| *held = (*held).min(first)).or_insert(first);
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
        Ok(Self { lines, rocket, country, rows, archetypes, archetype_since, equipment_observations, rocket_observations, repairs, span })
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

    pub(crate) fn equipment_coverage(&self) -> Vec<Piece<Amount>> {
        self.lines.iter().map(|&(first, last, context)| (first, last, context.amount(None, ProductionSpendingUnknown::SpendingUnavailable))).collect()
    }

    pub(super) fn rocket_total(&self) -> Vec<Piece<Amount>> {
        let mut total: Vec<Piece<Amount>> = self.rocket.iter().map(|&(first, last, context)| (first, last, context.amount(None, ProductionSpendingUnknown::SpendingUnavailable))).collect();
        for key in self.rows.keys().filter(|key| key.0) {
            total = zip(&total, &self.amounts(key, |row| row.spent), |a, b| a.plus(*b));
        }
        total
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
        Ok(Self::new(total, parts))
    }

    pub(crate) fn new(total: Vec<Piece<Option<(Option<i64>, bool)>>>, parts: BTreeMap<Option<u64>, Vec<Piece<Option<Option<i64>>>>>) -> Self {
        let mut read = Self { total, parts, index: breakdown::RepairIndex::default() };
        read.index = breakdown::RepairIndex::of(&read);
        read
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

    #[cfg(test)]
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

#[derive(Clone, Debug)]
pub(crate) struct Design {
    pub(crate) type_key: String,
    pub(crate) name: Option<String>,
    pub(crate) domain: Option<Domain>,
    pub(crate) names: Vec<StockVariantName>,
    pub(crate) typed_since: Option<i32>,
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
    let history = cost::storage(|| lanes::history(recording, span.run, TableId::VariantDefinitions, &DEFINITION_COLUMNS, Keys::These(&keys), span.first, span.last))?;
    let dated: HashMap<u64, DefinitionDates> = history.keys.iter().filter_map(|(key, steps)| definition_dates(&history, steps, span.first).map(|dates| (*key as u64, dates))).collect();
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
    tokens.extend(dated.values().flat_map(|dates| dates.names.iter().filter_map(|(_, fields)| fields.as_ref())).map(|fields| u128::from(fields.equipment_token)).filter(|&token| token > 0));
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
    let base = |variant: u64| definitions.get(&variant).cloned().unwrap_or_else(|| VariantDefinitionRow { id: variant, ..VariantDefinitionRow::default() });
    let naming_link = |fields: &NameFields| if (fields.origin_link as i32) >= 1 { fields.origin_link } else { fields.creator_link };
    let country_named = |fields: &NameFields| fields.display_name.is_empty() && naming_link(fields) != 0;
    let created = |fields: &NameFields| country_named(fields) && (fields.origin_link as i32) < 1;
    let mut entries: HashMap<u64, Vec<(i32, Option<NameFields>)>> = variants
        .keys()
        .filter_map(|&variant| {
            let held = match (dated.get(&variant).filter(|dates| !dates.names.is_empty()), definitions.get(&variant)) {
                (Some(dates), _) => dates.names.clone(),
                (None, Some(definition)) => vec![(span.last, Some(NameFields::from(definition)))],
                (None, None) => return None,
            };
            Some((variant, held))
        })
        .collect();
    let wanted: Vec<u128> = entries
        .values()
        .flatten()
        .filter_map(|(_, fields)| fields.as_ref())
        .filter(|fields| country_named(fields))
        .map(|fields| u128::from(naming_link(fields)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut mapped: HashMap<u32, Vec<(i32, i32, Option<u32>)>> = HashMap::new();
    let mut appearance: HashMap<u32, Vec<i32>> = HashMap::new();
    let mut appearance_coverage: Vec<i32> = Vec::new();
    let (mut links_unread, mut looks_unread): (Option<String>, Option<String>) = (None, None);
    if !wanted.is_empty() {
        let column = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
        let looks = [dashboard_declare::field(TableId::CountryAppearance, "name_tag")?, dashboard_declare::field(TableId::CountryAppearance, "original_tag")?];
        let creators = entries.values().flatten().filter_map(|(_, fields)| fields.as_ref()).any(created);
        let links_history = match cost::storage(|| lanes::history(recording, span.run, TableId::CountryLinks, &[column], if creators { Keys::All } else { Keys::These(&wanted) }, span.first, span.last)) {
            Ok(history) => Some(history),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                links_unread = Some(format!("Country links were not read for design names: {error}"));
                None
            }
        };
        let looks_history = match creators.then(|| cost::storage(|| lanes::history(recording, span.run, TableId::CountryAppearance, &looks, Keys::All, span.first, span.last))) {
            None => None,
            Some(Ok(history)) => Some(history),
            Some(Err(error)) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Some(Err(error)) => {
                looks_unread = Some(format!("Country appearance was not read for design names: {error}"));
                None
            }
        };
        let history = links_history.unwrap_or_default();
        for (key, steps) in &history.keys {
            let pieces = steps
                .iter()
                .enumerate()
                .map(|(at, step)| {
                    let last = steps.get(at + 1).map_or(history.to, |next| next.hour - 1);
                    let slot = step.cells.as_deref().and_then(|cells| match cells.first() {
                        Some(Cell::Number(value)) => u32::try_from(*value).ok(),
                        _ => None,
                    });
                    (step.hour, last, slot)
                })
                .collect();
            mapped.insert(*key as u32, pieces);
        }
        if let Some(looks) = &looks_history {
            for (key, steps) in &looks.keys {
                let pieces = mapped.get(&(*key as u32)).map_or(&[][..], Vec::as_slice);
                for step in steps {
                    let held = pieces.get(pieces.partition_point(|piece| piece.1 < step.hour)).filter(|piece| piece.0 <= step.hour).and_then(|piece| piece.2);
                    if let Some(slot) = held {
                        appearance.entry(slot).or_default().push(step.hour);
                    }
                }
            }
            appearance_coverage.extend(looks.statuses.iter().map(|status| status.first));
        }
    }
    for why in [&links_unread, &looks_unread].into_iter().flatten() {
        notes.insert(why.clone());
    }
    let withheld = |fields: &NameFields| country_named(fields) && (links_unread.is_some() || (looks_unread.is_some() && created(fields)));
    for held in entries.values_mut() {
        for entry in held.iter_mut() {
            if entry.1.as_ref().is_some_and(withheld) {
                entry.1 = None;
            }
        }
        let mut retagged: Vec<(i32, Option<NameFields>)> = Vec::new();
        for (at, (hour, fields)) in held.iter().enumerate() {
            let Some(fields) = fields.as_ref().filter(|fields| country_named(fields)) else { continue };
            let until = held.get(at + 1).map_or(span.last, |next| next.0 - 1);
            let mut edges: BTreeSet<i32> = BTreeSet::new();
            if created(fields) {
                edges.extend(appearance_coverage.iter().copied().filter(|&edge| edge > *hour && edge <= until));
            }
            for &(from, to, slot) in mapped.get(&naming_link(fields)).map_or(&[][..], Vec::as_slice) {
                let (low, high) = (from.max(*hour), to.min(until));
                if low > high {
                    continue;
                }
                if from > *hour {
                    edges.insert(from);
                }
                if let Some(slot) = slot {
                    for (first, last) in query.holders.naming_spans(&Country::Slot(slot)) {
                        edges.extend([first, last.saturating_add(1)].into_iter().filter(|&edge| edge > low && edge <= high));
                    }
                    if created(fields) {
                        edges.extend(appearance.get(&slot).into_iter().flatten().copied().filter(|&edge| edge > low && edge <= high));
                    }
                }
            }
            retagged.extend(edges.into_iter().map(|edge| (edge, Some(fields.clone()))));
        }
        held.extend(retagged);
        held.sort_by_key(|entry| entry.0);
        held.dedup_by_key(|entry| entry.0);
    }
    let mut by_hour: BTreeMap<i32, Vec<(u64, usize)>> = BTreeMap::new();
    for (&variant, held) in &entries {
        for (at, (hour, fields)) in held.iter().enumerate() {
            if fields.is_some() {
                by_hour.entry(*hour).or_default().push((variant, at));
            }
        }
    }
    let mut dated_names: HashMap<(u64, usize), StockVariantName> = HashMap::new();
    let mut frames = at_hour::Frames::new(query)?;
    for (hour, asked) in by_hour {
        let resolve = |row: &VariantDefinitionRow, shown: &Tags, lexical: &[LexicalTokenRow]| {
            let mut raw = variant_names::raw_variant(row, hour, shown, lexical);
            let own = of_variant(modules.as_deref(), row.id, |row: &VariantModuleRow| row.variant);
            let upgraded = of_variant(upgrades.as_deref(), row.id, |row: &VariantUpgradeRow| row.variant);
            if let (Some(own), Some(upgraded)) = (own, upgraded) {
                let own: Vec<&VariantModuleRow> = own.iter().collect();
                let upgraded: Vec<&VariantUpgradeRow> = upgraded.iter().collect();
                variant_names::with_design(&mut raw, &own, &upgraded, &known_spelling);
            }
            let (name, _) = variant_names::variant_names(&raw, texts);
            StockVariantName { first_hour: hour, name, known: true, design: raw.design }
        };
        let unknown = StockVariantName { first_hour: hour, name: None, known: false, design: None };
        let fields_of = |variant: u64, at: usize| entries[&variant][at].1.clone().unwrap_or_default();
        if hour == span.last {
            for (variant, at) in asked {
                let fields = fields_of(variant, at);
                let name = match &tags {
                    Ok(tags) => resolve(&fields.applied(&base(variant)), tags, lexical),
                    Err(_) if !country_named(&fields) => resolve(&fields.applied(&base(variant)), &empty, lexical),
                    Err(_) => unknown.clone(),
                };
                dated_names.insert((variant, at), name);
            }
            continue;
        }
        let read = [
            at_hour::Read::these(TableId::LexicalTokens, asked.iter().map(|&(variant, at)| u128::from(fields_of(variant, at).equipment_token)).filter(|&token| token > 0)),
            at_hour::Read::all(TableId::Countries),
            at_hour::Read::all(TableId::CountryLinks),
            at_hour::Read::all(TableId::CountryAppearance),
        ];
        let then = frames.frame(hour, &read, "Production spending").ok();
        let then_tags = then.as_ref().map(|frame| Tags::from_tables(HourFrame::from(frame), true).or_else(|_| Tags::from_tables(HourFrame::from(frame), false)));
        let then_lexical = then.as_ref().and_then(|frame| frame.complete::<tables::LexicalTokens>().ok()).unwrap_or(&[]);
        for (variant, at) in asked {
            let fields = fields_of(variant, at);
            let name = match &then_tags {
                Some(Ok(tags)) => resolve(&fields.applied(&base(variant)), tags, then_lexical),
                _ if !country_named(&fields) => resolve(&fields.applied(&base(variant)), &empty, then_lexical),
                _ => unknown.clone(),
            };
            dated_names.insert((variant, at), name);
        }
    }
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
        let names = match entries.get(&variant) {
            None => vec![StockVariantName { first_hour: span.first, name: None, known: false, design: None }],
            Some(held) => held
                .iter()
                .enumerate()
                .map(|(at, (hour, _))| dated_names.remove(&(variant, at)).unwrap_or(StockVariantName { first_hour: *hour, name: None, known: false, design: None }))
                .collect(),
        };
        let typed_since = definition.filter(|definition| definition.archetype_token > 0).and_then(|_| dated.get(&variant)).and_then(|dates| dates.typed);
        out.insert(variant, Design { type_key, name, domain, names, typed_since });
    }
    Ok(out)
}

const DEFINITION_COLUMNS: [usize; 8] = [0, 1, 2, 3, 7, 8, 9, 10];
const ARCHETYPE_AT: usize = 7;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DefinitionDates {
    pub(crate) since: i32,
    pub(crate) typed: Option<i32>,
    pub(crate) names: Vec<(i32, Option<NameFields>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NameFields {
    pub(crate) equipment_token: u32,
    pub(crate) creator_link: u32,
    pub(crate) origin_link: u32,
    pub(crate) append_variant_name: u8,
    pub(crate) display_name: Vec<u8>,
    pub(crate) variant_name: Vec<u8>,
    pub(crate) explicit_entity: Vec<u8>,
}

impl From<&VariantDefinitionRow> for NameFields {
    fn from(row: &VariantDefinitionRow) -> Self {
        Self {
            equipment_token: row.equipment_token,
            creator_link: row.creator_link,
            origin_link: row.origin_link,
            append_variant_name: row.append_variant_name,
            display_name: row.display_name.clone(),
            variant_name: row.variant_name.clone(),
            explicit_entity: row.explicit_entity.clone(),
        }
    }
}

impl NameFields {
    fn of(history: &History, cells: &[Cell]) -> Self {
        let number = |at: usize| match cells.get(at) {
            Some(Cell::Number(value)) => *value,
            _ => 0,
        };
        let text = |at: usize| cells.get(at).and_then(|cell| history.text(*cell)).map(<[u8]>::to_vec).unwrap_or_default();
        Self {
            equipment_token: u32::try_from(number(0)).unwrap_or(0),
            creator_link: u32::try_from(number(1)).unwrap_or(0),
            origin_link: u32::try_from(number(2)).unwrap_or(0),
            append_variant_name: u8::try_from(number(3)).unwrap_or(0),
            display_name: text(4),
            variant_name: text(5),
            explicit_entity: text(6),
        }
    }

    fn applied(&self, definition: &VariantDefinitionRow) -> VariantDefinitionRow {
        VariantDefinitionRow {
            equipment_token: self.equipment_token,
            creator_link: self.creator_link,
            origin_link: self.origin_link,
            append_variant_name: self.append_variant_name,
            display_name: self.display_name.clone(),
            variant_name: self.variant_name.clone(),
            explicit_entity: self.explicit_entity.clone(),
            ..definition.clone()
        }
    }
}

pub(crate) fn definition_dates(history: &History, steps: &[Step], floor: i32) -> Option<DefinitionDates> {
    let same = |a: &[Cell], b: &[Cell]| {
        a[..ARCHETYPE_AT].iter().zip(&b[..ARCHETYPE_AT]).all(|(&x, &y)| match (x, y) {
            (Cell::Text(_), Cell::Text(_)) => history.text(x) == history.text(y),
            _ => x == y,
        })
    };
    let mut dates: Option<DefinitionDates> = None;
    let mut last: Option<&[Cell]> = None;
    for step in steps {
        let hour = step.hour.max(floor);
        let Some(cells) = step.cells.as_deref() else {
            if let Some(held) = dates.as_mut().filter(|_| last.is_some()) {
                held.names.push((hour, None));
            }
            last = None;
            continue;
        };
        let typed = matches!(cells.get(ARCHETYPE_AT), Some(Cell::Number(token)) if *token > 0);
        let held = dates.get_or_insert_with(|| DefinitionDates { since: hour, typed: None, names: Vec::new() });
        if typed && held.typed.is_none() {
            held.typed = Some(hour);
        }
        if last.is_none_or(|before| !same(before, cells)) {
            held.names.push((hour, Some(NameFields::of(history, cells))));
        }
        last = Some(cells);
    }
    dates
}

fn of_variant<T>(rows: Option<&[T]>, variant: u64, key: impl Fn(&T) -> u64) -> Option<&[T]> {
    rows.map(|rows| &rows[rows.partition_point(|row| key(row) < variant)..rows.partition_point(|row| key(row) <= variant)])
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

pub(super) fn cached_rocket_name(source: &Source, catalogue: &Arc<Catalogue>) -> RocketName {
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

impl Product for ProductionSpending {
    const KEY: &'static str = "production-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionSpendingParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, fromHour: Start, hour: End, scope, includeRefits];
    type Reply = ProductionSpendingReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProductionSpendingScope::decl(), ProductionSpendingParams::decl(), ProductionSpendingReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        breakdown::load(source, params, context, cancelled)
    }
}
