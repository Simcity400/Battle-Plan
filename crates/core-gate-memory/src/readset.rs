use crate::{
    keyset::{self, Cache, Index, Keys},
    manifest::{self, Family, Fields, Selection, invalid},
    memory::cell,
};
use replay::columns;
use replay_columnar::{state::Cell, walk::State};
use replay_data::{format::FormatVersion, rows::TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Stream,
    Referenced,
    Presence,
    Snapshot,
    None,
}

#[derive(Clone, Copy, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Rows {
    #[default]
    All,
    EquipmentSlices,
    RelationSlices,
    Guarantees,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Hours {
    Every,
    DecisionCharge,
    LedgerDated,
    ScopedProjects,
}

impl Hours {
    pub fn name(self) -> &'static str {
        match self {
            Self::Every => "every",
            Self::DecisionCharge => "decision-charge",
            Self::LedgerDated => "ledger-dated",
            Self::ScopedProjects => "scoped-projects",
        }
    }
}

fn dated(hour: i32) -> u128 {
    u128::from((hour as u32) ^ (1 << 31))
}

struct HourIndex {
    table: TableId,
    rows: Index,
    dates: Index,
    status: Option<columns::Status>,
}

impl HourIndex {
    fn new(path: &Path, table: TableId, cache: &Cache) -> io::Result<Self> {
        Ok(Self {
            table,
            rows: Index::new(&scratch(path, "rows"), cache)?,
            dates: Index::new(&scratch(path, "dates"), cache)?,
            status: None,
        })
    }

    fn remove(&mut self, key: u128) -> io::Result<()> {
        if let Some(date) = self.rows.get((key, 0))? {
            self.dates.remove((date.into(), key))?;
            self.rows.remove((key, 0))?;
        }
        Ok(())
    }

    fn refresh(&mut self, state: &State, whole: bool, format: FormatVersion) -> io::Result<()> {
        let status = columns::status(state, self.table);
        let reset = whole || self.status != Some(status);
        self.status = Some(status);
        if !columns::complete(state, self.table) {
            if reset {
                self.rows.clear()?;
                self.dates.clear()?;
            }
            return Ok(());
        }
        let table = columns::table(state, self.table)?;
        let changes = columns::changes(state, self.table)?;
        let reset = reset || changes.truncated;
        if reset {
            self.rows.clear()?;
            self.dates.clear()?;
        } else {
            for &key in &changes.deleted_keys {
                self.remove(key)?;
            }
        }
        let mut update = |slot| -> io::Result<()> {
            let Some(key) = table.key(slot).filter(|_| table.live().get(slot)) else {
                return Ok(());
            };
            self.remove(key)?;
            if let Some(hour) = rule_date(state, self.table, slot, format)? {
                let date = dated(hour);
                self.rows.put((key, 0), date as u64)?;
                self.dates.put((date, key), 1)?;
            }
            Ok(())
        };
        if reset {
            for slot in table.live().ones() {
                update(slot)?;
            }
        } else {
            for &slot in &changes.inserted {
                update(slot)?;
            }
            for &slot in changes.changed.iter().chain(&changes.fill_slots) {
                update(slot)?;
            }
        }
        Ok(())
    }

    fn active(&self, hour: i32) -> io::Result<bool> {
        Ok(self
            .dates
            .first((dated(hour), 0))?
            .is_some_and(|(key, _)| key.0 == dated(hour)))
    }
}

fn rule_date(
    state: &State,
    id: TableId,
    slot: u32,
    format: FormatVersion,
) -> io::Result<Option<i32>> {
    use replay_data::{
        event_feed_rows::{EventFeedHistoryRow, EventFeedSource},
        rows::*,
    };
    Ok(match id {
        TableId::EventFeedHistory => {
            let row = columns::row::<EventFeedHistoryRow>(state, id, slot, format.version())?;
            row.events
                .iter()
                .any(|event| {
                    matches!(
                        event.source,
                        EventFeedSource::DecisionSelected | EventFeedSource::DecisionTimeoutEffect
                    )
                })
                .then_some(row.read_hour)
        }
        TableId::PoliticsCaptureCoverage => {
            let row =
                columns::row::<PoliticsCaptureCoverageRow>(state, id, slot, format.version())?;
            (row.since_hour == row.through_hour).then_some(row.since_hour)
        }
        TableId::CountryPowerChanges => {
            let row = columns::row::<CountryPowerChangeRow>(state, id, slot, format.version())?;
            let named = !matches!(
                row.cause,
                PowerCause::CapCorrection | PowerCause::ResetSetup | PowerCause::Console
            ) && !(row.cause == PowerCause::DailyGain
                && row.currency == PowerCurrency::Political);
            named.then_some(row.hour)
        }
        TableId::CountryPowerDailyCosts => {
            let row = columns::row::<CountryPowerDailyCostRow>(state, id, slot, format.version())?;
            (row.cost_raw != 0).then_some(row.day_hour)
        }
        _ => return Err(invalid("Unsupported hour-rule source")),
    })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub name: String,
    pub fields: Fields,
    pub spool: Mode,
    pub source: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub previous: bool,
    #[serde(default)]
    pub read_rule: Option<String>,
    #[serde(default)]
    pub filter: Rows,
    #[serde(default)]
    pub hours: Option<Hours>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Values {
    #[default]
    Scalar,
    Endpoints,
    RailwayTemplateToken,
    SubunitTokens,
    RequirementTokens,
    PowerToken,
    PowerCharacter,
    PowerOrganisation,
    PowerState,
    PowerLeader,
    Constant,
    OwnedScalar,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub from: String,
    pub to: String,
    pub source: String,
    #[serde(default)]
    pub values: Values,
    #[serde(default)]
    pub shift: u32,
    #[serde(default)]
    pub bits: Option<u32>,
    #[serde(default)]
    pub target_shift: u32,
    #[serde(default)]
    pub retained: bool,
    #[serde(default)]
    pub low_word: Option<u32>,
    #[serde(default)]
    pub constant: Option<String>,
    #[serde(default)]
    pub lookup_field: Option<String>,
}

#[derive(Clone)]
pub struct Entry {
    pub selected: Selection,
    pub mode: Mode,
    pub target_shift: u32,
    pub retained: bool,
    pub filter: Rows,
    pub hours: Option<Hours>,
}

#[derive(Clone)]
struct Edge {
    from: usize,
    to: usize,
    field: Option<usize>,
    reference: Reference,
    lookup: Option<usize>,
    constant: Option<u128>,
}

#[derive(Clone)]
pub struct ReadSet {
    pub tables: Vec<Entry>,
    edges: Vec<Edge>,
    order: Vec<usize>,
}

impl ReadSet {
    pub fn historical_tables(&self) -> Vec<bool> {
        let mut historical: Vec<_> = self
            .tables
            .iter()
            .map(|entry| entry.mode == Mode::Referenced && entry.selected.previous)
            .collect();
        for &at in &self.order {
            if historical[at] {
                for edge in self.edges.iter().filter(|edge| edge.from == at) {
                    historical[edge.to] = true;
                }
            }
        }
        historical
    }

    pub fn upper_bound(tables: &[Selection], hours: Hours) -> Self {
        Self {
            tables: tables
                .iter()
                .map(|selected| Entry {
                    selected: selected.clone(),
                    mode: Mode::Snapshot,
                    target_shift: 0,
                    retained: false,
                    filter: Rows::All,
                    hours: Some(hours),
                })
                .collect(),
            edges: Vec::new(),
            order: (0..tables.len()).collect(),
        }
    }
}

fn fields(table: TableId, fields: &Fields) -> io::Result<Vec<usize>> {
    match fields {
        Fields::All(all) if all == "all" => Ok((0..table.schema().len()).collect()),
        Fields::Named(names) => {
            let mut out = names
                .iter()
                .map(|name| columns::field(table, name))
                .collect::<io::Result<Vec<_>>>()?;
            out.sort_unstable();
            if out.windows(2).any(|w| w[0] == w[1]) {
                return Err(invalid("Repeated read-set field"));
            }
            Ok(out)
        }
        _ => Err(invalid("Read-set fields must be all or a list")),
    }
}

pub fn resolve(family: &Family, shared: &[Selection]) -> io::Result<Option<ReadSet>> {
    if !family.ordered {
        if !family.readset.is_empty() || !family.references.is_empty() {
            return Err(invalid("Only ordered families have spool read sets"));
        }
        return Ok(None);
    }
    let mut named = BTreeMap::new();
    let mut tables = Vec::new();
    for declared in &family.readset {
        let id = manifest::table(&declared.name)?;
        let original = family.tables.iter().find(|t| t.name == declared.name);
        let shared = shared.iter().find(|s| s.table == id);
        if original.is_none() && shared.is_none() {
            return Err(invalid(format!(
                "{}: read-set table {} has no family subscription",
                family.name, declared.name
            )));
        }
        let previous = declared.previous;
        if declared.source.is_empty()
            || declared.source == "undeclared"
            || named.insert(id, tables.len()).is_some()
        {
            return Err(invalid(format!(
                "{}: duplicate or unsourced read-set entry {}",
                family.name, declared.name
            )));
        }
        if declared.spool == Mode::None && previous {
            return Err(invalid("Previous fields cannot be omitted"));
        }
        if (declared.spool == Mode::Snapshot) != declared.hours.is_some() {
            return Err(invalid(
                "Snapshot tables require exactly one hour predicate",
            ));
        }
        if declared.spool == Mode::Presence && !fields(id, &declared.fields)?.is_empty() {
            return Err(invalid("Presence tables cannot carry row fields"));
        }
        if !matches!(
            (id, declared.filter),
            (_, Rows::All)
                | (
                    TableId::SliceReads,
                    Rows::EquipmentSlices | Rows::RelationSlices
                )
                | (TableId::RelationTuples, Rows::Guarantees)
        ) {
            return Err(invalid("Row predicate does not apply to this table"));
        }
        if declared.filter == Rows::RelationSlices && declared.hours != Some(Hours::Every) {
            return Err(invalid(
                "Current-hour slice reads require an hourly snapshot",
            ));
        }
        if declared.spool == Mode::Stream
            && matches!(declared.fields, Fields::All(_))
            && declared.reason.as_deref().is_none_or(str::is_empty)
        {
            return Err(invalid(format!(
                "{}: full-state spool {} needs a reason",
                family.name, declared.name
            )));
        }
        let read_rule = declared.read_rule.clone();
        if read_rule
            .as_deref()
            .is_some_and(|r| !matches!(r, "supplied" | "acquired"))
        {
            return Err(invalid("Invalid spool read rule"));
        }
        let mut subscription = match declared.spool {
            Mode::Stream => "stream",
            Mode::Referenced => "lookup",
            Mode::Presence => "presence",
            Mode::Snapshot => "snapshot",
            Mode::None => "",
        }
        .to_owned();
        if declared.read_rule.is_some() {
            subscription.push_str(",wake-on-read");
        }
        tables.push(Entry {
            selected: Selection {
                table: id,
                fields: fields(id, &declared.fields)?,
                recorded_only: Default::default(),
                undeclared: false,
                subscription,
                previous,
                read_rule,
            },
            mode: declared.spool,
            target_shift: 0,
            retained: false,
            filter: declared.filter,
            hours: declared.hours,
        });
    }
    if family
        .tables
        .iter()
        .any(|t| !family.readset.iter().any(|r| r.name == t.name))
    {
        return Err(invalid(format!(
            "{}: every family table needs a spool read-set entry",
            family.name
        )));
    }
    let mut edges = Vec::new();
    let mut incoming = BTreeMap::new();
    for reference in &family.references {
        let (from, member) = reference
            .from
            .split_once('.')
            .ok_or_else(|| invalid("Reference source must be Table.field"))?;
        let from = *named
            .get(&manifest::table(from)?)
            .ok_or_else(|| invalid("Unlisted reference source"))?;
        let to = *named
            .get(&manifest::table(&reference.to)?)
            .ok_or_else(|| invalid("Unlisted reference target"))?;
        let source = &tables[from];
        if source.mode == Mode::None
            || tables[to].mode != Mode::Referenced
            || reference.source.is_empty()
        {
            return Err(invalid(format!(
                "{}: reference must connect spooled fields to a referenced table",
                family.name
            )));
        }
        let field = if member == "$key" {
            None
        } else {
            let field = columns::field(source.selected.table, member)?;
            if !source.selected.fields.contains(&field) {
                return Err(invalid(format!(
                    "Unspooled reference field {}",
                    reference.from
                )));
            }
            Some(field)
        };
        let target_bits = tables[to].selected.table.key_bits();
        // A keyless (single-row) target takes no projection: only shift 0 addresses it.
        if reference.shift >= 128
            || (target_bits == 0 && reference.target_shift != 0)
            || (target_bits > 0 && reference.target_shift >= target_bits)
            || reference
                .bits
                .is_some_and(|n| n == 0 || n > 128 - reference.shift)
        {
            return Err(invalid("Reference key projection exceeds its width"));
        }
        match (reference.values, source.selected.table, member) {
            (Values::SubunitTokens, TableId::DivisionTemplateIcons, "subunits")
            | (Values::RequirementTokens, TableId::CountryLocalSubunits, "requirements")
            | (Values::RailwayTemplateToken, TableId::RailwayConstruction, "$key") => {}
            (
                Values::PowerToken
                | Values::PowerCharacter
                | Values::PowerOrganisation
                | Values::PowerState
                | Values::PowerLeader,
                TableId::CountryPowerChanges | TableId::CountryPowerDailyCosts,
                "identity",
            ) => {}
            (Values::Constant, _, _) if reference.constant.is_some() => {}
            (Values::Scalar | Values::Endpoints | Values::OwnedScalar, _, _) => {
                if let Some(field) = field {
                    use replay_columnar::state::Kind;
                    let kinds = replay::columnar_frames::schema()?;
                    if !matches!(
                        kinds[source.selected.table.index()][field],
                        Kind::U8
                            | Kind::U16
                            | Kind::U32
                            | Kind::U64
                            | Kind::I8
                            | Kind::I16
                            | Kind::I32
                            | Kind::I64
                            | Kind::OptionalU32
                            | Kind::OptionalU64
                            | Kind::OptionalI32
                            | Kind::OptionalI64
                            | Kind::U32List
                            | Kind::U64List
                    ) {
                        return Err(invalid(format!(
                            "Unsupported reference field {}",
                            reference.from
                        )));
                    }
                }
            }
            _ => {
                return Err(invalid(format!(
                    "Unsupported reference decoder {}",
                    reference.from
                )));
            }
        }
        let lookup = reference
            .lookup_field
            .as_ref()
            .map(|name| columns::field(tables[to].selected.table, name))
            .transpose()?;
        if let Some(field) = lookup {
            let kinds = replay::columnar_frames::schema()?;
            if reference.target_shift != 0
                || kinds[tables[to].selected.table.index()][field]
                    != replay_columnar::state::Kind::U32
            {
                return Err(invalid(
                    "Reverse lookup requires a U32 field and whole target keys",
                ));
            }
        }
        let shape = (reference.target_shift, reference.retained);
        if incoming.insert(to, shape).is_some_and(|old| old != shape) {
            return Err(invalid(
                "References to a table disagree on key shape or lifetime",
            ));
        }
        tables[to].target_shift = reference.target_shift;
        tables[to].retained = reference.retained;
        edges.push(Edge {
            from,
            to,
            field,
            reference: reference.clone(),
            lookup,
            constant: reference
                .constant
                .as_ref()
                .map(|value| {
                    value
                        .parse::<u128>()
                        .map_err(|_| invalid("Invalid constant reference key"))
                })
                .transpose()?,
        });
    }
    for (at, table) in tables.iter().enumerate() {
        if table.mode == Mode::Referenced && !incoming.contains_key(&at) {
            return Err(invalid(format!(
                "{}: {:?} has no declared reference",
                family.name, table.selected.table
            )));
        }
    }
    let mut done = BTreeSet::new();
    let mut order = Vec::new();
    while done.len() < tables.len() {
        let next = (0..tables.len())
            .find(|at| {
                !done.contains(at)
                    && edges
                        .iter()
                        .filter(|e| e.to == *at)
                        .all(|e| done.contains(&e.from))
            })
            .ok_or_else(|| invalid("Cyclic spool references"))?;
        done.insert(next);
        order.push(next);
    }
    let readset = ReadSet {
        tables,
        edges,
        order,
    };
    crate::proof::validate(family, &readset)?;
    Ok(Some(readset))
}

pub struct Filter {
    pub readset: ReadSet,
    keys: Vec<Option<Keys>>,
    rows: Vec<Option<Index>>,
    touched: Vec<Index>,
    links: Vec<Index>,
    format: FormatVersion,
    statuses: Vec<Option<columns::Status>>,
    active: Vec<bool>,
    hour_indexes: Vec<HourIndex>,
    lookups: Vec<Option<Lookup>>,
    historical: Historical,
}

struct HistoricalRows {
    live: Index,
    requested: Index,
    hydrated: Index,
}

struct HistoricalLookup {
    rows: Index,
    values: Index,
}

struct Historical {
    tables: Vec<Option<HistoricalRows>>,
    links: Vec<Option<Index>>,
    lookups: Vec<Option<HistoricalLookup>>,
    hour: Option<i32>,
}

impl HistoricalLookup {
    fn remove(&mut self, key: u128) -> io::Result<()> {
        if let Some(group) = self.values.get((key, 0))? {
            self.rows.remove((group.into(), key))?;
            self.values.remove((key, 0))?;
        }
        Ok(())
    }

    fn capture(&mut self, state: &State, id: TableId, field: usize, reset: bool) -> io::Result<()> {
        let table = columns::table(state, id)?;
        let changes = columns::changes(state, id)?;
        if reset {
            self.rows.clear()?;
            self.values.clear()?;
        } else {
            for &key in &changes.deleted_keys {
                self.remove(key)?;
            }
        }
        let mut update = |slot| -> io::Result<()> {
            let Some(key) = table.key(slot).filter(|_| table.live().get(slot)) else {
                return Ok(());
            };
            self.remove(key)?;
            if table.recorded(field, slot) {
                let Cell::U32(group) = cell(table, field, slot)? else {
                    return Err(invalid("Historical reverse lookup changed type"));
                };
                self.values.put((key, 0), group.into())?;
                self.rows.put((group.into(), key), 1)?;
            }
            Ok(())
        };
        if reset {
            for slot in table.live().ones() {
                update(slot)?;
            }
        } else {
            for &slot in &changes.inserted {
                update(slot)?;
            }
            for (&slot, &mask) in changes
                .changed
                .iter()
                .zip(&changes.written)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            {
                if mask & (1 << field) != 0 {
                    update(slot)?;
                }
            }
        }
        Ok(())
    }
}

impl Historical {
    fn new(path: &Path, readset: &ReadSet, cache: &Cache) -> io::Result<Self> {
        let needed = readset.historical_tables();
        Ok(Self {
            tables: needed
                .iter()
                .enumerate()
                .map(|(at, needed)| {
                    if !needed {
                        return Ok(None);
                    }
                    Ok(Some(HistoricalRows {
                        live: Index::new(&scratch(path, &format!("historical-{at}.live")), cache)?,
                        requested: Index::new(
                            &scratch(path, &format!("historical-{at}.requested")),
                            cache,
                        )?,
                        hydrated: Index::new(
                            &scratch(path, &format!("historical-{at}.hydrated")),
                            cache,
                        )?,
                    }))
                })
                .collect::<io::Result<_>>()?,
            links: readset
                .edges
                .iter()
                .enumerate()
                .map(|(at, edge)| {
                    needed[edge.from]
                        .then(|| {
                            Index::new(&scratch(path, &format!("historical-edge-{at}")), cache)
                        })
                        .transpose()
                })
                .collect::<io::Result<_>>()?,
            lookups: readset
                .edges
                .iter()
                .enumerate()
                .map(|(at, edge)| {
                    if !needed[edge.to] || edge.lookup.is_none() {
                        return Ok(None);
                    }
                    Ok(Some(HistoricalLookup {
                        rows: Index::new(
                            &scratch(path, &format!("historical-lookup-{at}.rows")),
                            cache,
                        )?,
                        values: Index::new(
                            &scratch(path, &format!("historical-lookup-{at}.values")),
                            cache,
                        )?,
                    }))
                })
                .collect::<io::Result<_>>()?,
            hour: None,
        })
    }

    fn request(&mut self, edge_at: usize, edge: &Edge, value: u128) -> io::Result<()> {
        let value = if let Some(lookup) = &self.lookups[edge_at] {
            let Some(((group, key), _)) = lookup.rows.first((value, 0))? else {
                return Ok(());
            };
            if group != value {
                return Ok(());
            }
            key
        } else {
            value
        };
        self.tables[edge.to]
            .as_mut()
            .unwrap()
            .requested
            .put((value, 0), 1)
    }

    fn close(
        &mut self,
        readset: &ReadSet,
        keys: &[Option<Keys>],
        lookups: &[Option<Lookup>],
        hour: i32,
    ) -> io::Result<()> {
        for table in self.tables.iter_mut().flatten() {
            table.requested.clear()?;
            table.hydrated.clear()?;
        }
        if self.hour.is_none() {
            return Ok(());
        }
        for (at, table) in self.tables.iter_mut().enumerate() {
            let Some(table) = table else {
                continue;
            };
            let mut next = Some(0);
            while let Some(start) = next {
                let Some((group, before, now)) = keys[at].as_ref().unwrap().next_change(start)?
                else {
                    break;
                };
                if now && !before {
                    table.requested.put((group, 0), 1)?;
                }
                next = group.checked_add(1);
            }
        }
        for (at, edge) in readset.edges.iter().enumerate() {
            if self.lookups[at].is_none() {
                continue;
            }
            let mut next = Some(0);
            while let Some(start) = next {
                let Some((group, before, now)) =
                    lookups[at].as_ref().unwrap().groups.next_change(start)?
                else {
                    break;
                };
                if now && !before {
                    self.request(at, edge, group)?;
                }
                next = group.checked_add(1);
            }
        }
        for &at in &readset.order {
            if self.tables[at].is_none() {
                continue;
            }
            let entry = &readset.tables[at];
            let mut next = Some((0, 0));
            while let Some(start) = next {
                let Some((request, _)) =
                    self.tables[at].as_ref().unwrap().requested.first(start)?
                else {
                    break;
                };
                if self.hour != hour.checked_sub(1) {
                    return Err(invalid(
                        "Historical reference closure has no frame at the supplying hour",
                    ));
                }
                let group = request.0;
                let mut row = Some((group, 0));
                let mut any = false;
                while let Some(start) = row {
                    let Some((key, _)) = self.tables[at].as_ref().unwrap().live.first(start)?
                    else {
                        break;
                    };
                    if key.0 != group {
                        break;
                    }
                    any = true;
                    self.tables[at]
                        .as_mut()
                        .unwrap()
                        .hydrated
                        .put((key.1, 0), 1)?;
                    for (edge_at, edge) in readset
                        .edges
                        .iter()
                        .enumerate()
                        .filter(|(_, edge)| edge.from == at)
                    {
                        let mut link = Some((key.1, 0));
                        while let Some(start) = link {
                            let Some((reference, _)) =
                                self.links[edge_at].as_ref().unwrap().first(start)?
                            else {
                                break;
                            };
                            if reference.0 != key.1 {
                                break;
                            }
                            self.request(edge_at, edge, reference.1)?;
                            link = keyset::after(reference);
                        }
                    }
                    row = keyset::after(key);
                }
                if !any && entry.target_shift == 0 {
                    self.tables[at]
                        .as_mut()
                        .unwrap()
                        .hydrated
                        .put((group, 0), 0)?;
                }
                next = keyset::after(request);
            }
        }
        Ok(())
    }

    fn capture(
        &mut self,
        state: &State,
        readset: &ReadSet,
        statuses: &[Option<columns::Status>],
        whole: bool,
        hour: i32,
        format: FormatVersion,
    ) -> io::Result<()> {
        for (at, previous) in self.tables.iter_mut().enumerate() {
            let Some(previous) = previous else {
                continue;
            };
            let entry = &readset.tables[at];
            let id = entry.selected.table;
            let table = columns::table(state, id)?;
            let changes = columns::changes(state, id)?;
            let reset =
                whole || changes.truncated || statuses[at] != Some(columns::status(state, id));
            if reset {
                previous.live.clear()?;
            } else {
                for &key in &changes.deleted_keys {
                    previous.live.remove((key >> entry.target_shift, key))?;
                }
            }
            let mut update = |slot| -> io::Result<()> {
                let Some(key) = table.key(slot).filter(|_| table.live().get(slot)) else {
                    return Ok(());
                };
                if entry.accepts(state, slot, hour)? {
                    previous.live.put((key >> entry.target_shift, key), 1)?;
                } else {
                    previous.live.remove((key >> entry.target_shift, key))?;
                }
                Ok(())
            };
            if reset {
                for slot in table.live().ones() {
                    update(slot)?;
                }
            } else {
                for &slot in changes
                    .inserted
                    .iter()
                    .chain(&changes.changed)
                    .chain(&changes.fill_slots)
                {
                    update(slot)?;
                }
            }
        }
        for (at, edge) in readset.edges.iter().enumerate() {
            if let Some(links) = &mut self.links[at] {
                let source = &readset.tables[edge.from];
                let id = source.selected.table;
                let table = columns::table(state, id)?;
                let changes = columns::changes(state, id)?;
                let reset = whole
                    || changes.truncated
                    || statuses[edge.from] != Some(columns::status(state, id));
                let mut remove = |key| -> io::Result<()> {
                    while let Some((link, _)) = links.first((key, 0))? {
                        if link.0 != key {
                            break;
                        }
                        links.remove(link)?;
                    }
                    Ok(())
                };
                if reset {
                    links.clear()?;
                } else {
                    for &key in &changes.deleted_keys {
                        remove(key)?;
                    }
                }
                let mut update = |slot| -> io::Result<()> {
                    let Some(key) = table.key(slot).filter(|_| table.live().get(slot)) else {
                        return Ok(());
                    };
                    while let Some((link, _)) = links.first((key, 0))? {
                        if link.0 != key {
                            break;
                        }
                        links.remove(link)?;
                    }
                    if !source.accepts(state, slot, hour)?
                        || edge.field.is_some_and(|field| !table.recorded(field, slot))
                    {
                        return Ok(());
                    }
                    references(state, id, slot, key, edge, format, &mut |value| {
                        links.put((key, value), 1)
                    })
                };
                if reset {
                    for slot in table.live().ones() {
                        update(slot)?;
                    }
                } else {
                    for &slot in &changes.inserted {
                        update(slot)?;
                    }
                    if let Some(field) = edge.field {
                        for (&slot, &mask) in changes
                            .changed
                            .iter()
                            .zip(&changes.written)
                            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
                        {
                            if mask & (1 << field) != 0 {
                                update(slot)?;
                            }
                        }
                    }
                }
            }
            if let Some(lookup) = &mut self.lookups[at] {
                let id = readset.tables[edge.to].selected.table;
                let reset = whole
                    || columns::changes(state, id)?.truncated
                    || statuses[edge.to] != Some(columns::status(state, id));
                lookup.capture(state, id, edge.lookup.unwrap(), reset)?;
            }
        }
        self.hour = Some(hour);
        Ok(())
    }
}

struct Lookup {
    groups: Keys,
    rows: Index,
    values: Index,
    chosen: Index,
    changed: Index,
}

impl Lookup {
    fn new(path: &Path, retained: bool, cache: &Cache) -> io::Result<Self> {
        Ok(Self {
            groups: Keys::new(&scratch(path, "groups"), retained, cache)?,
            rows: Index::new(&scratch(path, "rows"), cache)?,
            values: Index::new(&scratch(path, "values"), cache)?,
            chosen: Index::new(&scratch(path, "chosen"), cache)?,
            changed: Index::new(&scratch(path, "changed"), cache)?,
        })
    }

    fn remove(&mut self, key: u128) -> io::Result<()> {
        if let Some(group) = self.values.get((key, 0))? {
            self.rows.remove((group.into(), key))?;
            self.values.remove((key, 0))?;
            self.changed.put((group.into(), 0), 1)?;
        }
        Ok(())
    }

    fn refresh(
        &mut self,
        state: &State,
        id: TableId,
        field: usize,
        whole: bool,
        target: &mut Keys,
    ) -> io::Result<()> {
        self.changed.clear()?;
        let table = columns::table(state, id)?;
        let changes = columns::changes(state, id)?;
        if whole {
            while let Some((key, _)) = self.values.first((0, 0))? {
                self.remove(key.0)?;
            }
        } else {
            for &key in &changes.deleted_keys {
                self.remove(key)?;
            }
        }
        let mut update = |slot| -> io::Result<()> {
            let Some(key) = table.key(slot).filter(|_| table.live().get(slot)) else {
                return Ok(());
            };
            self.remove(key)?;
            if table.recorded(field, slot) {
                let Cell::U32(group) = cell(table, field, slot)? else {
                    return Err(invalid("Reverse lookup changed type"));
                };
                self.values.put((key, 0), group.into())?;
                self.rows.put((group.into(), key), 1)?;
                self.changed.put((group.into(), 0), 1)?;
            }
            Ok(())
        };
        if whole {
            for slot in table.live().ones() {
                update(slot)?;
            }
        } else {
            for &slot in &changes.inserted {
                update(slot)?;
            }
            for (&slot, &mask) in changes
                .changed
                .iter()
                .zip(&changes.written)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            {
                if mask & (1 << field) != 0 {
                    update(slot)?;
                }
            }
        }
        let mut next = Some(0);
        while let Some(start) = next {
            let Some((group, _, _)) = self.groups.next_change(start)? else {
                break;
            };
            self.changed.put((group, 0), 1)?;
            next = group.checked_add(1);
        }
        let mut next = Some((0, 0));
        while let Some(start) = next {
            let Some((key, _)) = self.changed.first(start)? else {
                break;
            };
            let group = key.0;
            let old = self
                .chosen
                .first((group, 0))?
                .map(|(key, _)| key)
                .filter(|key| key.0 == group);
            let new = if self.groups.contains(group)? {
                self.rows
                    .first((group, 0))?
                    .map(|(key, _)| key)
                    .filter(|key| key.0 == group)
            } else {
                None
            };
            if old != new {
                if let Some(key) = old {
                    target.adjust(key.1, false)?;
                    self.chosen.remove(key)?;
                }
                if let Some(key) = new {
                    target.adjust(key.1, true)?;
                    self.chosen.put(key, 1)?;
                }
            }
            next = keyset::after(key);
        }
        Ok(())
    }
}

fn scratch(path: &Path, suffix: &str) -> std::path::PathBuf {
    path.with_file_name(format!(
        "{}.{suffix}",
        path.file_name().unwrap().to_string_lossy()
    ))
}

fn remove_source(links: &mut Index, keys: &mut Keys, source: u128) -> io::Result<()> {
    while let Some(((row, target), _)) = links.first((source, 0))? {
        if row != source {
            break;
        }
        keys.adjust(target, false)?;
        links.remove((row, target))?;
    }
    Ok(())
}

impl Entry {
    pub fn accepts_key(&self, key: u128) -> bool {
        match self.filter {
            Rows::All => true,
            Rows::EquipmentSlices => key >> 32 == TableId::CombatLogEquipment as u128,
            Rows::RelationSlices => key >> 32 == TableId::RelationTuples as u128,
            Rows::Guarantees => (key >> 16) as u32 == engine_native::cguarantee_relation::TYPE,
        }
    }

    pub fn accepts(&self, state: &State, slot: u32, hour: i32) -> io::Result<bool> {
        let table = columns::table(state, self.selected.table)?;
        let key = table
            .key(slot)
            .ok_or_else(|| invalid("Filtered row has no key"))?;
        if !self.accepts_key(key) {
            return Ok(false);
        }
        if self.filter == Rows::RelationSlices {
            let field = columns::field(self.selected.table, "sample_hour")?;
            return Ok(matches!(cell(table, field, slot)?, Cell::I32(sample) if sample == hour));
        }
        Ok(true)
    }
}

impl Filter {
    pub fn active(&self, at: usize) -> bool {
        self.active[at]
    }
    pub fn new(
        path: &Path,
        readset: &ReadSet,
        format: FormatVersion,
        cache: &Cache,
    ) -> io::Result<Self> {
        let keys = readset
            .tables
            .iter()
            .enumerate()
            .map(|(at, t)| {
                if t.mode == Mode::Referenced {
                    Keys::new(&scratch(path, &format!("{at}")), t.retained, cache).map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<io::Result<_>>()?;
        let rows = readset
            .tables
            .iter()
            .enumerate()
            .map(|(at, t)| {
                if t.mode == Mode::Referenced {
                    Index::new(&scratch(path, &format!("{at}.rows")), cache).map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            historical: Historical::new(path, readset, cache)?,
            readset: readset.clone(),
            keys,
            rows,
            touched: (0..readset.tables.len())
                .map(|at| Index::new(&scratch(path, &format!("{at}.touched")), cache))
                .collect::<io::Result<_>>()?,
            links: (0..readset.edges.len())
                .map(|at| Index::new(&scratch(path, &format!("edge-{at}")), cache))
                .collect::<io::Result<_>>()?,
            format,
            statuses: vec![None; readset.tables.len()],
            active: vec![true; readset.tables.len()],
            hour_indexes: [
                (Hours::DecisionCharge, TableId::EventFeedHistory),
                (Hours::LedgerDated, TableId::PoliticsCaptureCoverage),
                (Hours::LedgerDated, TableId::CountryPowerChanges),
                (Hours::LedgerDated, TableId::CountryPowerDailyCosts),
            ]
            .into_iter()
            .filter(|(rule, _)| {
                readset
                    .tables
                    .iter()
                    .any(|entry| entry.hours == Some(*rule))
            })
            .map(|(_, id)| HourIndex::new(&scratch(path, &format!("hours-{id:?}")), id, cache))
            .collect::<io::Result<_>>()?,
            lookups: readset
                .edges
                .iter()
                .enumerate()
                .map(|(at, edge)| {
                    edge.lookup
                        .map(|_| {
                            Lookup::new(
                                &scratch(path, &format!("lookup-{at}")),
                                edge.reference.retained,
                                cache,
                            )
                        })
                        .transpose()
                })
                .collect::<io::Result<_>>()?,
        })
    }

    pub fn refresh(&mut self, state: &State, whole: bool, hour: i32) -> io::Result<()> {
        for index in &mut self.hour_indexes {
            index.refresh(state, whole, self.format)?;
        }
        let mut hours = [None; 4];
        for (at, entry) in self.readset.tables.iter().enumerate() {
            self.active[at] = if let Some(rule) = entry.hours {
                match hours[rule as usize] {
                    Some(active) => active,
                    None => {
                        let active = match rule {
                            Hours::Every | Hours::ScopedProjects => true,
                            Hours::DecisionCharge => {
                                let table = columns::table(state, TableId::EventFeedHistory)?;
                                let mut live = table.live().ones();
                                columns::complete(state, TableId::EventFeedHistory)
                                    && live.next().is_some()
                                    && live.next().is_none()
                                    && self.hour_indexes[0].active(hour)?
                            }
                            Hours::LedgerDated => {
                                let mut active =
                                    !columns::complete(state, TableId::PoliticsCaptureCoverage);
                                for index in &self.hour_indexes {
                                    if index.table != TableId::EventFeedHistory {
                                        active |= index.active(hour)?;
                                    }
                                }
                                active
                            }
                        };
                        hours[rule as usize] = Some(active);
                        active
                    }
                }
            } else {
                true
            };
        }
        for keys in self.keys.iter_mut().flatten() {
            keys.begin()?;
        }
        for lookup in self.lookups.iter_mut().flatten() {
            lookup.groups.begin()?;
        }
        for touched in &mut self.touched {
            touched.clear()?;
        }
        for &at in &self.readset.order {
            let entry = &self.readset.tables[at];
            let table = columns::table(state, entry.selected.table)?;
            let changed = columns::changes(state, entry.selected.table)?;
            if let Some(rows) = &mut self.rows[at] {
                if whole {
                    rows.clear()?;
                    for slot in table.live().ones() {
                        let key = table
                            .key(slot)
                            .ok_or_else(|| invalid("Missing indexed key"))?;
                        rows.put((key >> entry.target_shift, key), 1)?;
                    }
                } else {
                    for &key in &changed.deleted_keys {
                        rows.remove((key >> entry.target_shift, key))?;
                    }
                    for &slot in &changed.inserted {
                        let key = table
                            .key(slot)
                            .ok_or_else(|| invalid("Missing inserted key"))?;
                        rows.put((key >> entry.target_shift, key), 1)?;
                    }
                }
            }
            if entry.mode == Mode::Referenced {
                let mut target = self.keys[at].take().unwrap();
                for (edge_at, edge) in self
                    .readset
                    .edges
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.to == at)
                {
                    let mut lookup = self.lookups[edge_at].take();
                    let destination = if let Some(lookup) = &mut lookup {
                        &mut lookup.groups
                    } else {
                        &mut target
                    };
                    let source = &self.readset.tables[edge.from];
                    let source_table = columns::table(state, source.selected.table)?;
                    let changes = columns::changes(state, source.selected.table)?;
                    let status = columns::status(state, source.selected.table);
                    let reset =
                        whole || changes.truncated || self.statuses[edge.from] != Some(status);
                    let links = &mut self.links[edge_at];
                    if reset {
                        while let Some(((source, _), _)) = links.first((0, 0))? {
                            remove_source(links, destination, source)?;
                        }
                    } else {
                        for &key in &changes.deleted_keys {
                            remove_source(links, destination, key)?;
                        }
                    }
                    let mut update = |slot: u32| -> io::Result<()> {
                        let Some(key) = source_table
                            .key(slot)
                            .filter(|_| source_table.live().get(slot))
                        else {
                            return Ok(());
                        };
                        remove_source(links, destination, key)?;
                        let included = source.accepts(state, slot, hour)?
                            && self.keys[edge.from].as_ref().map_or(Ok(true), |keys| {
                                keys.contains(key >> source.target_shift)
                            })?;
                        if !included
                            || edge
                                .field
                                .is_some_and(|field| !source_table.recorded(field, slot))
                        {
                            return Ok(());
                        }
                        references(
                            state,
                            source.selected.table,
                            slot,
                            key,
                            edge,
                            self.format,
                            &mut |value| {
                                if links.get((key, value))?.is_none() {
                                    links.put((key, value), 1)?;
                                    destination.adjust(value, true)?;
                                }
                                Ok(())
                            },
                        )
                    };
                    if reset {
                        for slot in source_table.live().ones() {
                            update(slot)?;
                        }
                    } else {
                        for &slot in &changes.inserted {
                            update(slot)?;
                        }
                        if let Some(field) = edge.field {
                            for (&slot, &mask) in changes
                                .changed
                                .iter()
                                .zip(&changes.written)
                                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
                            {
                                if mask & (1_u64 << field) != 0 {
                                    update(slot)?;
                                }
                            }
                        }
                        let mut next = Some((0, 0));
                        while let Some(start) = next {
                            let Some((key, _)) = self.touched[edge.from].first(start)? else {
                                break;
                            };
                            if let Some(slot) = source_table.slot(key.0) {
                                update(slot)?;
                            }
                            next = keyset::after(key);
                        }
                    }
                    if let Some(lookup) = &mut lookup {
                        lookup.refresh(
                            state,
                            entry.selected.table,
                            edge.lookup.unwrap(),
                            whole,
                            &mut target,
                        )?;
                    }
                    self.lookups[edge_at] = lookup;
                }
                let mut next = Some(0);
                while let Some(start) = next {
                    let Some((group, _, _)) = target.next_change(start)? else {
                        break;
                    };
                    let mut row = Some((group, 0));
                    while let Some(start) = row {
                        let Some((key, _)) = self.rows[at].as_ref().unwrap().first(start)? else {
                            break;
                        };
                        if key.0 != group {
                            break;
                        }
                        self.touched[at].put((key.1, 0), 1)?;
                        row = keyset::after(key);
                    }
                    next = group.checked_add(1);
                }
                self.keys[at] = Some(target);
            }
        }
        self.historical
            .close(&self.readset, &self.keys, &self.lookups, hour)?;
        self.historical.capture(
            state,
            &self.readset,
            &self.statuses,
            whole,
            hour,
            self.format,
        )?;
        for (at, entry) in self.readset.tables.iter().enumerate() {
            self.statuses[at] = Some(columns::status(state, entry.selected.table));
        }
        Ok(())
    }

    pub fn contains(&self, at: usize, key: u128) -> io::Result<bool> {
        let entry = &self.readset.tables[at];
        if !entry.accepts_key(key) {
            return Ok(false);
        }
        match &self.keys[at] {
            Some(keys) => keys.contains(key >> entry.target_shift),
            None => Ok(matches!(entry.mode, Mode::Stream | Mode::Snapshot)),
        }
    }

    pub fn contained(&self, at: usize, key: u128) -> io::Result<bool> {
        let entry = &self.readset.tables[at];
        if !entry.accepts_key(key) {
            return Ok(false);
        }
        match &self.keys[at] {
            Some(keys) => keys.contained(key >> entry.target_shift),
            None => Ok(matches!(entry.mode, Mode::Stream | Mode::Snapshot)),
        }
    }

    pub fn historical_contains(&self, at: usize, key: u128) -> io::Result<bool> {
        let entry = &self.readset.tables[at];
        Ok(entry.accepts_key(key)
            && self.historical.tables[at]
                .as_ref()
                .unwrap()
                .requested
                .get((key >> entry.target_shift, 0))?
                .is_some())
    }

    pub fn has_historical(&self, at: usize) -> io::Result<bool> {
        self.historical.tables[at]
            .as_ref()
            .map_or(Ok(false), |table| {
                Ok(table.requested.first((0, 0))?.is_some())
            })
    }

    pub fn next_historical(&self, at: usize, start: u128) -> io::Result<Option<(u128, bool)>> {
        Ok(self.historical.tables[at]
            .as_ref()
            .unwrap()
            .hydrated
            .first((start, 0))?
            .map(|(key, present)| (key.0, present != 0)))
    }

    pub fn next_membership(&self, at: usize, start: u128) -> io::Result<Option<u128>> {
        Ok(self.touched[at].first((start, 0))?.map(|(key, _)| key.0))
    }
}

fn references(
    state: &State,
    id: TableId,
    slot: u32,
    key: u128,
    edge: &Edge,
    format: FormatVersion,
    output: &mut impl FnMut(u128) -> io::Result<()>,
) -> io::Result<()> {
    let reference = &edge.reference;
    let mut emit = |value: u128| {
        if let Some(expected) = reference.low_word {
            if value as u32 != expected {
                return Ok(());
            }
        }
        let value = if reference.values == Values::OwnedScalar {
            (key << 32) | value
        } else {
            value
        };
        let value = value >> reference.shift;
        let value = match reference.bits {
            Some(bits) if bits < 128 => value & ((1_u128 << bits) - 1),
            _ => value,
        };
        output(value)
    };
    match reference.values {
        Values::Constant => emit(
            edge.constant
                .ok_or_else(|| invalid("Missing constant reference"))?,
        ),
        Values::PowerToken
        | Values::PowerCharacter
        | Values::PowerOrganisation
        | Values::PowerState
        | Values::PowerLeader => {
            use replay_data::{rows::PowerObjectKind, value::Value};
            let Cell::Opaque(bytes) = cell(columns::table(state, id)?, edge.field.unwrap(), slot)?
            else {
                return Err(invalid("Power identity is not opaque"));
            };
            let mut input = replay_data::codec::Decoder::new(bytes);
            input.format = format.version();
            let identity = replay_data::rows::PowerIdentity::decode(&mut input)?;
            input.finish()?;
            let value = match reference.values {
                Values::PowerToken if identity.item_key.is_empty() => {
                    identity.item_token.map(u128::from)
                }
                Values::PowerCharacter => identity.character.map(u128::from),
                Values::PowerOrganisation
                    if identity.object_kind == PowerObjectKind::Organisation =>
                {
                    identity.object_reference.map(u128::from)
                }
                Values::PowerLeader if identity.object_kind == PowerObjectKind::Leader => {
                    identity.object_reference.map(u128::from)
                }
                Values::PowerState => identity.target_state.map(u128::from),
                _ => None,
            };
            if let Some(value) = value {
                emit(value)?;
            }
            Ok(())
        }
        Values::RailwayTemplateToken => emit(u128::from(
            engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN,
        )),
        Values::SubunitTokens => token_list::<replay_data::rows::TemplateSubunit>(
            state,
            id,
            slot,
            edge.field.unwrap(),
            format,
            |unit| emit(unit.token.into()),
        ),
        Values::RequirementTokens => token_list::<replay_data::rows::LocalSubunitRequirement>(
            state,
            id,
            slot,
            edge.field.unwrap(),
            format,
            |requirement| emit(requirement.archetype_token.into()),
        ),
        Values::Scalar | Values::Endpoints | Values::OwnedScalar => {
            let Some(field) = edge.field else {
                return emit(key);
            };
            match cell(columns::table(state, id)?, field, slot)? {
                Cell::U8(v) => emit(v.into()),
                Cell::U16(v) => emit(v.into()),
                Cell::U32(v) => emit(v.into()),
                Cell::U64(v) => emit(v.into()),
                Cell::I8(v) => emit((v as u8).into()),
                Cell::I16(v) => emit((v as u16).into()),
                Cell::I32(v) => emit((v as u32).into()),
                Cell::I64(v) => emit((v as u64).into()),
                Cell::OptionalI32(Some(v)) if v >= 0 => emit(v as u128),
                Cell::OptionalI64(Some(v)) if v >= 0 => emit(v as u128),
                Cell::OptionalU32(Some(v)) => emit(v.into()),
                Cell::OptionalU64(Some(v)) => emit(v.into()),
                Cell::OptionalI32(_)
                | Cell::OptionalI64(_)
                | Cell::OptionalU32(None)
                | Cell::OptionalU64(None) => Ok(()),
                Cell::U32List(list) => {
                    if reference.values == Values::Endpoints {
                        for v in list.first().into_iter().chain(list.last()) {
                            emit((*v).into())?;
                        }
                    } else {
                        for &v in list {
                            emit(v.into())?;
                        }
                    }
                    Ok(())
                }
                Cell::U64List(list) => {
                    for &v in list {
                        emit(v.into())?;
                    }
                    Ok(())
                }
                _ => Err(invalid(format!("Unsupported reference {}", reference.from))),
            }
        }
    }
}

fn token_list<T: replay_data::value::Value>(
    state: &State,
    id: TableId,
    slot: u32,
    field: usize,
    format: FormatVersion,
    mut emit: impl FnMut(T) -> io::Result<()>,
) -> io::Result<()> {
    let Cell::Opaque(bytes) = cell(columns::table(state, id)?, field, slot)? else {
        return Err(invalid("Reference list is not opaque"));
    };
    let mut input = replay_data::codec::Decoder::new(bytes);
    input.format = format.version();
    let count = input.count(T::MINIMUM_ENCODED_BYTES)?;
    for _ in 0..count {
        emit(T::decode(&mut input)?)?;
    }
    input.finish()
}
