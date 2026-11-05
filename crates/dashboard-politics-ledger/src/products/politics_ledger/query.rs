//! The ledger's replies, read from its series publication ([`super::emit`]): a country's rows
//! of the asked hours, as the emit stored them for replies ([`super::records`]), with the items
//! they name, each resolved once by the emit at its first payment. A reply decodes, filters and
//! groups them; it names, resolves and formats nothing. Totals read each settled period's rollup
//! whole ([`super::tally`]) and only the hours no rollup holds row by row.

use super::emit::{
    self, CATEGORY_NAMES, COSTS, COVER_REASONS, COVERED, DAILY, GOVERNMENTS, HOLDS, ITEMS, LATE,
    PARTS, POWER, PRESENT, ROLLUP,
};
use super::records::{CostRecord, DailyRecord, ItemRecord, PowerRecord};
use super::tally::{self, Contribution, Field};
use super::*;
use dashboard_base::prepared::codec;
use dashboard_base::prepared::engine;
use dashboard_base::prepared::series::{self, MeasureKey, Navigation};
use dashboard_base::prepared_chart::{MarkCluster, MarkSummary};
use dashboard_base::prepared_measure::query::{MeasureDropReply, MeasureEvent};
use dashboard_base::products::country_groups::HourSpan;
use dashboard_base::products::{MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES, validate_tag};
use dashboard_core::transport::json_len;
use dashboard_series::{
    Cell, Event, EventOrder, codec::Binary as _, EventQuery, PageRead, Selection, Snapshot, Span,
    family::{Directory, Scalar},
    query::EventView,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    view: LedgerView,
    from: Option<i32>,
    hour: i32,
    within: Option<Vec<HourSpan>>,
    navigation: Option<Navigation>,
    summary: Option<PartView>,
    #[serde(default)]
    totals: bool,
    bindings: Option<BTreeMap<String, Vec<HourSpan>>>,
    marks: Option<MarkView>,
    page: Option<GovernmentPage>,
    #[serde(default)]
    matches: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MarkView {
    from: f64,
    to: f64,
    width: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GovernmentPage {
    from: f64,
    to: f64,
    cursor: String,
    page_size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PartView {
    from: f64,
    to: f64,
    width: u32,
    currency: String,
    mode: String,
}

fn invalid(what: impl Into<String>) -> io::Error {
    dashboard_core::invalid(what.into())
}

fn cancelled_request(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Politics request cancelled",
        ))
    } else {
        Ok(())
    }
}

/// The reply before the family publishes the branch: the spending view's is empty, the rest
/// pending.
pub fn unpublished(params: &Value) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params.clone())?;
    validate_tag(&params.tag).map_err(dashboard_core::invalid)?;
    if params.view == LedgerView::Spending {
        return Ok(serde_json::to_value(PoliticsLedgerReply {
            branch_id: params.branch_id,
            tag: params.tag,
            decimals: 5,
            ..Default::default()
        })?);
    }
    Ok(Value::Null)
}

/// One publication's reader.
struct Reader<'r, 's, P> {
    snapshot: &'r mut Snapshot<'s, P>,
    directory: &'r Directory,
    /// The branch's first prepared hour and the publication's last.
    first: i32,
    last: i32,
    settled: Option<i32>,
    /// The buffer every record is read into.
    scratch: Vec<u8>,
}

/// An event's cells.
struct Cells(Vec<Cell>);

/// The hour an event was read at.
fn read_of(event: &Event) -> i32 {
    event.read_hour.unwrap_or(event.native_hour)
}

fn currency(command: bool) -> &'static str {
    if command { "Command" } else { "Political" }
}

impl<'r, 's, P: PageRead> Reader<'r, 's, P> {
    fn cells(&mut self, blob: dashboard_series::Blob) -> io::Result<Cells> {
        let count = blob.len / Cell::SIZE as u64;
        let mut cells = Vec::with_capacity(count as usize);
        for at in 0..count {
            cells.push(self.snapshot.field(blob, at)?);
        }
        Ok(Cells(cells))
    }

    /// The record cell `at` of `fields` holds ([`super::records`]).
    fn record<T: DeserializeOwned>(
        &mut self,
        fields: dashboard_series::Blob,
        at: u64,
    ) -> io::Result<T> {
        let Cell::Text(blob) = self.snapshot.field(fields, at)? else {
            return Err(invalid("A politics ledger record is malformed"));
        };
        let length =
            usize::try_from(blob.len).map_err(|_| invalid("A politics ledger record is too long"))?;
        self.scratch.clear();
        self.scratch.resize(length, 0);
        if length != 0 {
            self.snapshot.cell_bytes(blob, 0, &mut self.scratch)?;
        }
        codec::decode(&self.scratch)
    }

    /// `key`'s events of native hours `first..=last`, in native order.
    fn events(&mut self, key: Option<u64>, first: i32, last: i32) -> io::Result<Vec<Event>> {
        let mut found = Vec::new();
        let Some(key) = key else {
            return Ok(found);
        };
        if first > last {
            return Ok(found);
        }
        let selection = [Selection {
            key,
            membership: None,
        }];
        self.snapshot.each_event(
            EventQuery {
                selection: &selection,
                native: Span { first, last },
                as_of: None,
                order: EventOrder::Native,
            },
            BUDGET,
            |view| {
                if let EventView::Family(event) = view {
                    found.push(event);
                }
                Ok(())
            },
        )?;
        found.sort_by_key(|event| (event.native_hour, event.sequence));
        Ok(found)
    }

    /// `key`'s latest event at or before `hour`.
    fn latest(&mut self, key: Option<u64>, hour: i32) -> io::Result<Option<Event>> {
        let Some(key) = key else {
            return Ok(None);
        };
        Ok(
            match self.snapshot.latest_event(
                Selection {
                    key,
                    membership: None,
                },
                hour,
                None,
            )? {
                Some(EventView::Family(event)) => Some(event),
                _ => None,
            },
        )
    }

    fn runs(
        &mut self,
        key: Option<u64>,
        first: i32,
        last: i32,
    ) -> io::Result<Vec<dashboard_series::Run>> {
        match key {
            Some(key) => engine::runs(self.snapshot, key, first, last),
            None => Ok(Vec::new()),
        }
    }

    fn scalar_reasons(&mut self, id: u64) -> BTreeMap<String, u64> {
        let mut reasons = BTreeMap::new();
        if let Some(scalar) = self.directory.scalar(id) {
            for pair in scalar.list() {
                if let [reason, count] = pair.list()
                    && let (Some(reason), Some(count)) = (reason.text(), count.integer())
                {
                    reasons.insert(
                        String::from_utf8_lossy(reason).into_owned(),
                        u64::try_from(count).unwrap_or(0),
                    );
                }
            }
        }
        reasons
    }

    /// The recorded content's power warnings, once an item reached them.
    fn power_warnings(&self) -> Vec<String> {
        self.directory
            .scalar(emit::POWER_WARNINGS)
            .map(|scalar| {
                scalar
                    .list()
                    .iter()
                    .filter_map(|warning| warning.text())
                    .map(|warning| String::from_utf8_lossy(warning).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The keys of one country.
struct Keys {
    actor: Option<u64>,
}

impl Keys {
    fn of(directory: &Directory, tag: &str) -> Self {
        Self {
            actor: directory.actor(tag.as_bytes()),
        }
    }

    fn key(&self, directory: &Directory, measure: u64) -> Option<u64> {
        directory.key(self.actor?, measure, None)
    }

    fn part(&self, directory: &Directory, dimension: u64) -> Option<u64> {
        directory.key(self.actor?, PARTS, Some(dimension))
    }
}

/// An item as its payments show it: its item, category and tooltip.
struct Paid {
    item: LedgerSpendItem,
    category: LedgerSpendCategory,
    ideas: Option<LedgerIdeaTooltips>,
}

/// An item of the country: as the emit stored it, and as its payments show it.
struct Shown {
    stored: ItemRecord,
    paid: Arc<Paid>,
}

impl Shown {
    fn of(stored: ItemRecord) -> Self {
        let mut item = stored.item.clone();
        let mut category = stored.category.clone();
        if stored.ability_upkeep {
            item.name = None;
            item.icon = None;
            item.icon_art = None;
            item.tooltip_token = None;
            item.character = None;
            item.unknown = None;
            item.action = "Ability upkeep".into();
        }
        if let Some(daily) = &stored.daily {
            category = daily.clone();
            item.action = format!(
                "{}: {}",
                category.1,
                item.name.as_deref().unwrap_or("Not identified")
            );
        }
        let ideas = stored
            .ideas
            .clone()
            .filter(|_| item.tooltip_token.is_some());
        if item.tooltip_token.is_some() {
            item.icon = None;
        }
        Self {
            paid: Arc::new(Paid {
                item,
                category: LedgerSpendCategory {
                    identified: category.0 != "notIdentified",
                    id: category.0,
                    name: category.1,
                },
                ideas,
            }),
            stored,
        }
    }
}

/// A payment, as preparation recorded one and a reply groups them.
#[derive(Clone)]
struct Payment {
    spend: LedgerSpend,
    paid: Arc<Paid>,
    count: u32,
    /// The sum of its recorded charges, and how many are recorded.
    known: i128,
    known_count: u32,
    /// Its capture identity: country and epoch.
    identity: (u32, u64),
    ordinal: u32,
}

/// The item a payment groups under, of the capture identity that recorded it.
fn item_field(payment: &Payment) -> String {
    format!(
        "{}:{}:{}",
        payment.ordinal, payment.identity.0, payment.identity.1
    )
}

fn overflow() -> io::Error {
    invalid("Politics subtotal overflow")
}

fn count_overflow() -> io::Error {
    invalid("Politics payment count overflow")
}

fn add_raw(left: &mut Option<String>, right: Option<&str>) -> io::Result<()> {
    *left = match (rows::amount(left.as_deref())?, rows::amount(right)?) {
        (Some(left), Some(right)) => Some(left.checked_add(right).ok_or_else(overflow)?.to_string()),
        _ => None,
    };
    Ok(())
}

/// `right` merged into `left`, one open subtotal keeping both sides' contributions.
fn merge(left: &mut Payment, right: &Payment) -> io::Result<()> {
    left.spend
        .changes
        .extend(right.spend.changes.iter().cloned());
    add_raw(
        &mut left.spend.charge_raw,
        right.spend.charge_raw.as_deref(),
    )?;
    add_raw(
        &mut left.spend.realized_delta_raw,
        right.spend.realized_delta_raw.as_deref(),
    )?;
    left.spend.occurrences = left
        .spend
        .occurrences
        .checked_add(right.spend.occurrences)
        .ok_or_else(count_overflow)?;
    left.spend.covered &= right.spend.covered;
    if left.spend.category != "upkeep" {
        left.count = left
            .count
            .checked_add(right.count)
            .ok_or_else(count_overflow)?;
    }
    left.known_count = left
        .known_count
        .checked_add(right.known_count)
        .ok_or_else(count_overflow)?;
    left.known = left.known.checked_add(right.known).ok_or_else(overflow)?;
    left.spend.read_hour = left.spend.read_hour.max(right.spend.read_hour);
    if right.spend.hour < left.spend.hour {
        left.spend.hour = right.spend.hour;
        left.spend.country_name = None;
    }
    Ok(())
}

fn normalize_spans(within: &[HourSpan], from: i32, to: i32) -> Vec<HourSpan> {
    let mut values: Vec<_> = within
        .iter()
        .filter_map(|span| {
            let first_hour = span.first_hour.max(from);
            let last_hour = span.last_hour.min(to);
            (first_hour <= last_hour).then_some(HourSpan {
                first_hour,
                last_hour,
            })
        })
        .collect();
    values.sort_by_key(|span| span.first_hour);
    let mut result: Vec<HourSpan> = Vec::new();
    for span in values {
        if let Some(last) = result
            .last_mut()
            .filter(|last| i64::from(last.last_hour) + 1 >= i64::from(span.first_hour))
        {
            last.last_hour = last.last_hour.max(span.last_hour);
        } else {
            result.push(span);
        }
    }
    result
}

fn selected(spans: &[HourSpan], hour: i32) -> bool {
    spans
        .iter()
        .any(|span| span.first_hour <= hour && hour <= span.last_hour)
}

fn first_day(hour: i32) -> i64 {
    (i64::from(hour) + 23).div_euclid(24) * 24
}

fn last_day(hour: i32) -> i64 {
    i64::from(hour).div_euclid(24) * 24
}

/// The game month of `hour` (none before the calendar's epoch) and the hour its next starts.
fn month(hour: i32) -> (Option<i32>, i64) {
    use engine_native::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    if hour < HOUR_EPOCH {
        return (None, i64::from(hour) + 24);
    }
    let days = (i64::from(hour) - i64::from(HOUR_EPOCH)).div_euclid(24);
    let year = days.div_euclid(365);
    let mut day = days.rem_euclid(365);
    let mut index = 0;
    while day >= i64::from(MONTH_LENGTHS[index]) {
        day -= i64::from(MONTH_LENGTHS[index]);
        index += 1;
    }
    (
        Some((year * 12 + index as i64) as i32),
        i64::from(hour) - i64::from(hour).rem_euclid(24)
            + (i64::from(MONTH_LENGTHS[index]) - day) * 24,
    )
}

/// The first hour of the month window of `spans` that holds native day `day`: the month's first
/// whole day the first span reaching into it holds; an undated day is its own window.
fn month_window_start(spans: &[HourSpan], day: i32) -> i32 {
    use engine_native::cdate::MONTH_LENGTHS;
    let (month, next) = month(day);
    let Some(month) = month else {
        return day;
    };
    let length = i64::from(MONTH_LENGTHS[month.rem_euclid(12) as usize]);
    let (start, end) = (next - length * 24, next - 24);
    for span in spans {
        let first = first_day(span.first_hour).max(start);
        let last = last_day(span.last_hour).min(end);
        if first <= last {
            return first as i32;
        }
    }
    day
}

/// A payment of the country, as its presentation made it, and what totals count of it.
struct Presented {
    payment: Payment,
    contribution: Contribution,
    recurring: bool,
    late: bool,
}

/// The presentation of the country's payments in the hours a reply reads.
struct Presentation {
    payments: Vec<Presented>,
    reasons: BTreeMap<String, u64>,
    formats: BTreeSet<String>,
    /// The items whose formatting warnings `formats` holds.
    formatted: HashSet<u32>,
    /// Whether any power row shows a balance line of its item.
    balanced: bool,
}

impl Presentation {
    fn new() -> Self {
        Self {
            payments: Vec::new(),
            reasons: BTreeMap::new(),
            formats: BTreeSet::new(),
            formatted: HashSet::new(),
            balanced: false,
        }
    }

    /// Item `ordinal` shown: the warnings formatting it gave.
    fn show(&mut self, ordinal: u32, shown: &Shown) {
        if self.formatted.insert(ordinal) {
            self.formats.extend(shown.stored.formats.iter().cloned());
        }
    }
}

/// A recorded row read after its native day settled, as a totals reply reads it.
enum Late {
    Power((i32, u64, PowerRecord)),
    Cost((i32, u64, CostRecord)),
}

struct Country {
    tag: String,
    keys: Keys,
    /// Power rows and costs with their read hour and event sequence, in the order recorded.
    power: Vec<(i32, u64, PowerRecord)>,
    costs: Vec<(i32, u64, CostRecord)>,
    daily: Vec<DailyRecord>,
    items: HashMap<u32, Shown>,
}

impl Country {
    /// The country's rows of native hours within `windows`, and the items they show.
    fn read<P: PageRead>(
        reader: &mut Reader<'_, '_, P>,
        tag: &str,
        windows: &[(i32, i32)],
    ) -> io::Result<Self> {
        let keys = Keys::of(reader.directory, tag);
        let (mut power, mut costs, mut daily) = (Vec::new(), Vec::new(), Vec::new());
        for &(first, last) in windows {
            for measure in POWER {
                let key = keys.key(reader.directory, measure);
                for event in reader.events(key, first, last)? {
                    let record: PowerRecord = reader.record(event.fields, 0)?;
                    power.push((read_of(&event), event.sequence, record));
                }
            }
            for event in reader.events(keys.key(reader.directory, COSTS), first, last)? {
                let record: CostRecord = reader.record(event.fields, 0)?;
                costs.push((read_of(&event), event.sequence, record));
            }
            for event in reader.events(keys.key(reader.directory, DAILY), first, last)? {
                daily.push(reader.record::<DailyRecord>(event.fields, 0)?);
            }
        }
        power.sort_by_key(|(read, sequence, _)| (*read, *sequence));
        costs.sort_by_key(|(read, sequence, _)| (*read, *sequence));
        daily.sort_by_key(|row| (row.daily.day_hour, row.daily.read_hour));
        let mut country = Self {
            tag: tag.to_owned(),
            keys,
            power,
            costs,
            daily,
            items: HashMap::new(),
        };
        let wanted: BTreeSet<u32> = country
            .power
            .iter()
            .filter(|(_, _, record)| record.class != emit::SKIPPED)
            .map(|(_, _, record)| record.item)
            .chain(
                country
                    .costs
                    .iter()
                    .filter(|(_, _, record)| shows_item(record))
                    .map(|(_, _, record)| record.item),
            )
            .collect();
        country.load(reader, &wanted)?;
        Ok(country)
    }

    /// Reads the items of `wanted` not yet read from the country's catalogue.
    fn load<P: PageRead>(
        &mut self,
        reader: &mut Reader<'_, '_, P>,
        wanted: &BTreeSet<u32>,
    ) -> io::Result<()> {
        let missing: HashSet<u32> = wanted
            .iter()
            .copied()
            .filter(|ordinal| !self.items.contains_key(ordinal))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let key = self.keys.key(reader.directory, ITEMS);
        for event in reader.events(key, i32::MIN, i32::MAX)? {
            let ordinal = match reader.snapshot.field(event.fields, 0)? {
                Cell::Unsigned(ordinal) => u32::try_from(ordinal).ok(),
                _ => None,
            }
            .ok_or_else(|| invalid("A politics ledger item is malformed"))?;
            if !missing.contains(&ordinal) || self.items.contains_key(&ordinal) {
                continue;
            }
            let stored: ItemRecord = reader.record(event.fields, 1)?;
            self.items.insert(ordinal, Shown::of(stored));
        }
        if missing.iter().any(|ordinal| !self.items.contains_key(ordinal)) {
            return Err(invalid(
                "A politics payment's item is missing from its catalogue",
            ));
        }
        Ok(())
    }

    fn item(&self, ordinal: u32) -> io::Result<&Shown> {
        self.items
            .get(&ordinal)
            .ok_or_else(|| invalid("A politics payment's item is missing from its catalogue"))
    }

    /// A power row read in the reply's hours: its item shown, and its payment when it is one.
    fn present_power(
        &self,
        (read, sequence, record): &(i32, u64, PowerRecord),
        presentation: &mut Presentation,
    ) -> io::Result<Option<Presented>> {
        if record.class == emit::SKIPPED {
            return Ok(None);
        }
        let shown = self.item(record.item)?;
        presentation.show(record.item, shown);
        if let Some(reason) = &shown.stored.item.unknown {
            rows::count_reason(&mut presentation.reasons, reason.clone(), 1)?;
        }
        presentation.balanced = true;
        if record.class != emit::PAYMENT {
            return Ok(None);
        }
        let charge = record.charge.map(|raw| raw.to_string());
        let realized = Some(record.realized.to_string());
        let spend = LedgerSpend {
            country_id: record.country,
            id: rows::mutation_id(record.country, record.epoch, record.sequence),
            sequence: record.sequence.to_string(),
            ordinal: 0,
            hour: record.hour,
            read_hour: *read,
            country_tag: self.tag.clone(),
            country_name: shown.stored.country_name.clone(),
            currency: currency(record.command).into(),
            category: shown.paid.category.id.clone(),
            item_id: shown.stored.item.id.clone(),
            charge_raw: charge.clone(),
            realized_delta_raw: realized.clone(),
            covered: record.covered,
            occurrences: 1,
            changes: vec![LedgerSpendChange {
                hour: Some(record.hour),
                read_hour: *read,
                charge_raw: charge,
                realized_delta_raw: realized,
                occurrences: 1,
                covered: record.covered,
            }],
        };
        Ok(Some(Presented {
            contribution: Contribution {
                field: (record.item, record.country, record.epoch),
                recurring: record.recurring,
                upkeep: record.upkeep,
                hour: record.hour,
                read: *read,
                order: (*read, *sequence),
                sequence: record.sequence,
                ordinal: 0,
                charge: record.charge,
                covered: record.covered,
                realized: Some(record.realized),
            },
            payment: Payment {
                spend,
                paid: Arc::clone(&shown.paid),
                count: 1,
                known: record.charge.unwrap_or(0),
                known_count: u32::from(record.charge.is_some()),
                identity: (record.country, record.epoch),
                ordinal: record.item,
            },
            recurring: record.recurring,
            late: record.late,
        }))
    }

    /// A cost first recorded in the reply's hours, as the payment it is when it is one.
    fn present_cost(
        &self,
        (read, sequence, record): &(i32, u64, CostRecord),
        presentation: &mut Presentation,
    ) -> io::Result<Option<Presented>> {
        if record.class == emit::UNMATCHED {
            rows::count_reason(
                &mut presentation.reasons,
                "Daily political power cost: matched PP application is unavailable".into(),
                1,
            )?;
            return Ok(None);
        }
        let shown = self.item(record.item)?;
        presentation.show(record.item, shown);
        if record.class == emit::CREDIT {
            let name = shown.stored.item.name.as_deref().unwrap_or("Not identified");
            rows::count_reason(
                &mut presentation.reasons,
                format!(
                    "Negative daily political power cost component {name}: recorded credit is not included in spending"
                ),
                1,
            )?;
            return Ok(None);
        }
        if let Some(reason) = &shown.stored.item.unknown {
            rows::count_reason(&mut presentation.reasons, reason.clone(), 1)?;
        }
        let charge = Some(i128::from(record.cost_raw).to_string());
        let spend = LedgerSpend {
            country_id: record.country,
            id: rows::cost_id(record.country, record.epoch, record.day_hour, record.ordinal),
            sequence: record.pp_sequence.to_string(),
            ordinal: record.ordinal,
            hour: record.day_hour,
            read_hour: *read,
            country_tag: self.tag.clone(),
            country_name: shown.stored.country_name.clone(),
            currency: "Political".into(),
            category: shown.paid.category.id.clone(),
            item_id: shown.stored.item.id.clone(),
            charge_raw: charge.clone(),
            realized_delta_raw: None,
            covered: record.covered,
            occurrences: 1,
            changes: vec![LedgerSpendChange {
                hour: Some(record.day_hour),
                read_hour: *read,
                charge_raw: charge,
                realized_delta_raw: None,
                occurrences: 1,
                covered: record.covered,
            }],
        };
        Ok(Some(Presented {
            contribution: Contribution {
                field: (record.item, record.country, record.epoch),
                recurring: record.recurring,
                upkeep: record.upkeep,
                hour: record.day_hour,
                read: *read,
                order: (*read, *sequence),
                sequence: record.pp_sequence,
                ordinal: record.ordinal,
                charge: Some(i128::from(record.cost_raw)),
                covered: record.covered,
                realized: None,
            },
            payment: Payment {
                spend,
                paid: Arc::clone(&shown.paid),
                count: 1,
                known: i128::from(record.cost_raw),
                known_count: 1,
                identity: (record.country, record.epoch),
                ordinal: record.item,
            },
            recurring: record.recurring,
            late: record.late,
        }))
    }

    /// The country's payments read, in the order they were recorded.
    fn present<P: PageRead>(
        &self,
        reader: &Reader<'_, '_, P>,
        cancelled: &AtomicBool,
    ) -> io::Result<Presentation> {
        let mut presentation = Presentation::new();
        for row in &self.power {
            cancelled_request(cancelled)?;
            if let Some(presented) = self.present_power(row, &mut presentation)? {
                presentation.payments.push(presented);
            }
        }
        for row in &self.costs {
            cancelled_request(cancelled)?;
            if let Some(presented) = self.present_cost(row, &mut presentation)? {
                presentation.payments.push(presented);
            }
        }
        presentation
            .payments
            .sort_by_key(|presented| presented.contribution.order);
        if !presentation.payments.is_empty() || presentation.balanced {
            presentation.formats.extend(reader.power_warnings());
        }
        Ok(presentation)
    }

    /// A payment of item `field` no row of the reply's hours holds: named by its item, every
    /// amount and date left to the subtotal made of it.
    fn template(&self, field: Field) -> io::Result<Payment> {
        let shown = self.item(field.0)?;
        Ok(Payment {
            spend: LedgerSpend {
                country_id: field.1,
                id: String::new(),
                sequence: String::new(),
                ordinal: 0,
                hour: 0,
                read_hour: 0,
                country_tag: self.tag.clone(),
                country_name: shown.stored.country_name.clone(),
                currency: shown.stored.currency.clone(),
                category: shown.paid.category.id.clone(),
                item_id: shown.stored.item.id.clone(),
                charge_raw: None,
                realized_delta_raw: None,
                covered: true,
                occurrences: 0,
                changes: Vec::new(),
            },
            paid: Arc::clone(&shown.paid),
            count: 0,
            known: 0,
            known_count: 0,
            identity: (field.1, field.2),
            ordinal: field.0,
        })
    }
}

/// Whether a cost shows its item: one matched to its application.
fn shows_item(record: &CostRecord) -> bool {
    record.class != emit::UNMATCHED
}

/// The items a reply holds: each item, its category and its tooltip once.
#[derive(Default)]
struct Appended(HashSet<u32>);

/// `payment` in `reply`, as a spend or a recurring subtotal over `recurring` (first hour, last
/// hour, days); its item, category and tooltip the first time.
fn append_payment(
    reply: &mut PoliticsLedgerReply,
    appended: &mut Appended,
    payment: Payment,
    recurring: Option<(i32, i32, u32)>,
) -> io::Result<()> {
    if appended.0.insert(payment.ordinal) {
        let paid = &*payment.paid;
        if !reply.spend_items.iter().any(|item| item.id == paid.item.id) {
            reply.spend_items.push(paid.item.clone());
        }
        if !reply
            .spend_categories
            .iter()
            .any(|category| category.id == paid.category.id)
        {
            reply.spend_categories.push(paid.category.clone());
        }
        if let Some(ideas) = &paid.ideas {
            let target = reply
                .idea_tooltips
                .get_or_insert_with(|| LedgerIdeaTooltips {
                    ideas: Vec::new(),
                    artwork: Vec::new(),
                    modifiers: Vec::new(),
                    texts: Vec::new(),
                });
            for info in &ideas.ideas {
                if !target.ideas.iter().any(|old| old.token == info.token) {
                    target.ideas.push(info.clone());
                }
            }
            for art in &ideas.artwork {
                if !target.artwork.iter().any(|old| old.token == art.token) {
                    target.artwork.push(art.clone());
                }
            }
            for modifier in &ideas.modifiers {
                if !target.modifiers.iter().any(|old| old.key == modifier.key) {
                    target.modifiers.push(modifier.clone());
                }
            }
            for text in &ideas.texts {
                if !target.texts.iter().any(|old| old.key == text.key) {
                    target.texts.push(text.clone());
                }
            }
        }
    }
    match recurring {
        Some((first_hour, last_hour, days)) => reply.recurring.push(LedgerRecurringSpend {
            spend: payment.spend,
            first_hour,
            last_hour,
            days,
            count: payment.count,
            known_raw: payment.known.to_string(),
            known_count: payment.known_count,
        }),
        None => reply.spending.push(payment.spend),
    }
    Ok(())
}

/// `payment` shown on its own: one change, its whole subtotal.
fn alone(mut payment: Payment) -> Payment {
    payment.spend.changes = vec![LedgerSpendChange {
        hour: Some(payment.spend.hour),
        read_hour: payment.spend.read_hour,
        charge_raw: payment.spend.charge_raw.clone(),
        realized_delta_raw: payment.spend.realized_delta_raw.clone(),
        occurrences: payment.spend.occurrences,
        covered: payment.spend.covered,
    }];
    payment
}

/// One settled day of a recurring item: what its payments add up to, read at its latest
/// contribution.
struct Day {
    hour: i32,
    lag: i32,
    /// Its first payment's.
    sequence: u64,
    ordinal: u32,
    occurrences: u32,
    count: u32,
    known: i128,
    known_count: u32,
    covered: bool,
    realized_unknown: bool,
    realized: (i128, i128),
    /// A command day's payments merged: a span cutting the day shows what it holds of them.
    merged: Option<Payment>,
}

impl Day {
    fn new(presented: &Presented, day: i32, command: bool) -> Self {
        let payment = &presented.payment;
        let realized = presented.contribution.realized;
        Self {
            hour: payment.spend.hour,
            lag: payment.spend.read_hour - day,
            sequence: presented.contribution.sequence,
            ordinal: payment.spend.ordinal,
            occurrences: payment.spend.occurrences,
            count: payment.count,
            known: payment.known,
            known_count: payment.known_count,
            covered: payment.spend.covered,
            realized_unknown: realized.is_none(),
            realized: realized.map_or((0, 0), |value| (value.max(0), value.min(0))),
            merged: command.then(|| payment.clone()),
        }
    }

    /// Another payment of the day.
    fn add(&mut self, presented: &Presented, day: i32) -> io::Result<()> {
        let payment = &presented.payment;
        self.occurrences = self
            .occurrences
            .checked_add(payment.spend.occurrences)
            .ok_or_else(count_overflow)?;
        if payment.spend.category != "upkeep" {
            self.count = self
                .count
                .checked_add(payment.count)
                .ok_or_else(count_overflow)?;
        }
        self.known_count = self
            .known_count
            .checked_add(payment.known_count)
            .ok_or_else(count_overflow)?;
        self.known = self.known.checked_add(payment.known).ok_or_else(overflow)?;
        self.covered &= payment.spend.covered;
        self.lag = self.lag.max(payment.spend.read_hour - day);
        match presented.contribution.realized {
            Some(value) => {
                self.realized.0 += value.max(0);
                self.realized.1 += value.min(0);
            }
            None => self.realized_unknown = true,
        }
        self.hour = self.hour.min(payment.spend.hour);
        if let Some(merged) = &mut self.merged {
            merge(merged, payment)?;
        }
        Ok(())
    }
}

/// A recurring item's settled days by native day, and the payment its subtotals are made of.
struct Recurring {
    field: String,
    template: Payment,
    days: BTreeMap<i32, Day>,
}

/// `payment` with only its contributions read by `to` and dated within `spans`, totalled again.
fn clip_command_payment(
    mut payment: Payment,
    spans: &[HourSpan],
    to: i32,
) -> io::Result<Option<Payment>> {
    payment.spend.changes.retain(|change| {
        change.read_hour <= to && change.hour.is_some_and(|hour| selected(spans, hour))
    });
    if payment.spend.changes.is_empty() {
        return Ok(None);
    }
    let first = payment
        .spend
        .changes
        .iter()
        .filter_map(|change| change.hour)
        .min()
        .unwrap_or(payment.spend.hour);
    if first != payment.spend.hour {
        payment.spend.country_name = None;
    }
    payment.spend.hour = first;
    payment.spend.read_hour = payment
        .spend
        .changes
        .iter()
        .map(|change| change.read_hour)
        .max()
        .unwrap_or(payment.spend.read_hour);
    let charges = payment
        .spend
        .changes
        .iter()
        .map(|change| rows::amount(change.charge_raw.as_deref()))
        .collect::<io::Result<Vec<_>>>()?;
    payment.spend.charge_raw = rows::sum_raw(charges.into_iter())?.map(|value| value.to_string());
    let realized = payment
        .spend
        .changes
        .iter()
        .map(|change| rows::amount(change.realized_delta_raw.as_deref()))
        .collect::<io::Result<Vec<_>>>()?;
    payment.spend.realized_delta_raw =
        rows::sum_raw(realized.into_iter())?.map(|value| value.to_string());
    payment.spend.occurrences = payment
        .spend
        .changes
        .iter()
        .map(|change| change.occurrences)
        .sum();
    payment.known_count = payment
        .spend
        .changes
        .iter()
        .filter(|change| change.charge_raw.is_some())
        .map(|change| change.occurrences)
        .sum();
    let mut known = 0_i128;
    for change in &payment.spend.changes {
        if let Some(raw) = rows::amount(change.charge_raw.as_deref())? {
            known = known.checked_add(raw).ok_or_else(overflow)?;
        }
    }
    payment.known = known;
    payment.spend.covered = payment.spend.changes.iter().all(|change| change.covered);
    Ok(Some(payment))
}

/// What the country's payments add to a reply over `spans`, read by `to`: one-off spends, and
/// recurring subtotals by game month for settled days, by day for open or late ones.
#[allow(clippy::too_many_arguments)]
fn payments(
    reply: &mut PoliticsLedgerReply,
    appended: &mut Appended,
    presented: Vec<Presented>,
    settled: Option<i32>,
    view: &LedgerView,
    from: i32,
    to: i32,
    spans: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let boundary_days = boundary_days(spans);
    let settled_day = |day: i32| settled.is_some_and(|settled| day < settled);
    let mut fields = HashMap::<Field, usize>::new();
    let mut items = Vec::<Recurring>::new();
    let mut open = BTreeMap::<(i32, String), Payment>::new();
    let mut late = Vec::new();
    for presented in presented {
        cancelled_request(cancelled)?;
        let payment = &presented.payment;
        if !presented.recurring {
            if payment.spend.read_hour >= from
                && payment.spend.read_hour <= to
                && selected(spans, payment.spend.hour)
            {
                append_payment(reply, appended, presented.payment, None)?;
            }
            continue;
        }
        let day = payment.spend.hour.div_euclid(24) * 24;
        if presented.late {
            if payment.spend.read_hour <= to && selected(spans, payment.spend.hour) {
                late.push(presented.payment);
            }
            continue;
        }
        if !settled_day(day) {
            match open.entry((day, item_field(payment))) {
                std::collections::btree_map::Entry::Occupied(mut held) => {
                    merge(held.get_mut(), payment)?;
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(presented.payment);
                }
            }
            continue;
        }
        let at = *fields.entry(presented.contribution.field).or_insert_with(|| {
            items.push(Recurring {
                field: item_field(payment),
                template: payment.clone(),
                days: BTreeMap::new(),
            });
            items.len() - 1
        });
        let recurring = &mut items[at];
        let command = recurring.template.spend.currency == "Command";
        match recurring.days.entry(day) {
            std::collections::btree_map::Entry::Occupied(mut held) => {
                held.get_mut().add(&presented, day)?;
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Day::new(&presented, day, command));
            }
        }
    }
    items.sort_by(|left, right| left.field.cmp(&right.field));
    let mut category_days = BTreeMap::<(String, String), BTreeSet<i32>>::new();
    let mut cut = Vec::new();
    for recurring in &items {
        cancelled_request(cancelled)?;
        let template = &recurring.template;
        let command = template.spend.currency == "Command";
        // A settled day is read once its latest contribution is; Command days a span cuts are
        // shown on their own.
        let visible: Vec<i32> = recurring
            .days
            .iter()
            .filter(|(day, held)| {
                let day = **day;
                selected(spans, day)
                    && i64::from(day) + i64::from(held.lag) <= i64::from(to)
                    && !(command && boundary_days.contains(&day))
            })
            .map(|(day, _)| *day)
            .collect();
        if command {
            for (day, held) in &recurring.days {
                if boundary_days.contains(day)
                    && let Some(merged) = &held.merged
                    && let Some(clipped) = clip_command_payment(merged.clone(), spans, to)?
                {
                    cut.push(clipped);
                }
            }
        }
        category_days
            .entry((
                template.spend.currency.clone(),
                template.spend.category.clone(),
            ))
            .or_default()
            .extend(visible.iter().copied());
        let windows: Vec<(Option<i32>, Vec<i32>)> = {
            let mut by_month = BTreeMap::<Option<i32>, Vec<i32>>::new();
            let mut undated = Vec::new();
            for day in visible {
                match month(day).0 {
                    Some(month) => by_month.entry(Some(month)).or_default().push(day),
                    None => undated.push((None, vec![day])),
                }
            }
            undated.extend(by_month);
            undated
        };
        for (month, days) in windows {
            let Some(&native_hour) = days.last() else {
                continue;
            };
            let first_hour = month_window_start(spans, days[0]);
            let latest = &recurring.days[&native_hour];
            let read_hour = native_hour.saturating_add(latest.lag);
            let mut payment = template.clone();
            let (mut count, mut entries, mut charge, mut unknown, mut uncovered) =
                (0_u32, 0_u32, 0_i128, 0_u32, 0_u32);
            let (mut realized_unknown, mut realized) = (false, (0_i128, 0_i128));
            for day in &days {
                let held = &recurring.days[day];
                count += held.occurrences;
                entries += held.count;
                charge = charge.checked_add(held.known).ok_or_else(overflow)?;
                unknown += held.occurrences - held.known_count;
                uncovered += u32::from(!held.covered);
                realized_unknown |= held.realized_unknown;
                realized.0 += held.realized.0;
                realized.1 += held.realized.1;
            }
            payment.spend.id = format!(
                "recurring:{}:{}:{month:?}:{first_hour}",
                reply.tag, recurring.field
            );
            payment.spend.hour = latest.hour;
            payment.spend.read_hour = read_hour;
            payment.spend.sequence = latest.sequence.to_string();
            payment.spend.ordinal = latest.ordinal;
            payment.spend.charge_raw = (unknown == 0).then(|| charge.to_string());
            payment.known = charge;
            payment.known_count = count - unknown;
            payment.spend.realized_delta_raw = None;
            if (*view == LedgerView::Balance || month.is_none())
                && !realized_unknown
            {
                payment.spend.realized_delta_raw = Some((realized.0 + realized.1).to_string());
            }
            payment.spend.occurrences = count;
            payment.spend.covered = uncovered == 0;
            payment.count = entries;
            append_payment(
                reply,
                appended,
                alone(payment),
                Some((first_hour, native_hour, days.len() as u32)),
            )?;
        }
    }
    late.extend(cut);
    // Open days: their payments as read by `to`, merged per day; Command days a span cuts keep
    // only what the span holds.
    for ((day, _), payment) in open {
        let payment = if payment.spend.currency == "Command" && boundary_days.contains(&day) {
            match clip_command_payment(payment, spans, to)? {
                Some(clipped) => clipped,
                None => continue,
            }
        } else if payment.spend.read_hour <= to && selected(spans, payment.spend.hour) {
            payment
        } else {
            continue;
        };
        late.push(payment);
    }
    for payment in late {
        let day = payment.spend.hour.div_euclid(24) * 24;
        category_days
            .entry((
                payment.spend.currency.clone(),
                payment.spend.category.clone(),
            ))
            .or_default()
            .insert(day);
        append_payment(reply, appended, alone(payment), Some((day, day, 1)))?;
    }
    for ((currency, category), days) in category_days {
        let mut months = BTreeMap::<Option<i32>, u32>::new();
        for day in days {
            *months.entry(month(day).0).or_default() += 1;
        }
        reply
            .recurring_days
            .extend(months.into_iter().map(|(month, days)| LedgerMonthDays {
                currency: currency.clone(),
                category: category.clone(),
                month,
                days,
            }));
    }
    Ok(())
}

/// The latest revision of each of `rows`' days within `spans` read by `to`.
fn latest_daily(rows: &[DailyRecord], spans: &[HourSpan], to: i32) -> Vec<LedgerDaily> {
    let mut latest = BTreeMap::<(u32, u64, i32), &LedgerDaily>::new();
    for row in rows {
        let daily = &row.daily;
        if daily.read_hour > to || !selected(spans, daily.day_hour) {
            continue;
        }
        latest.insert((daily.country, row.epoch, daily.day_hour), daily);
    }
    let mut daily: Vec<LedgerDaily> = latest.into_values().cloned().collect();
    daily.sort_by_key(|row| (row.day_hour, row.country, row.instance));
    daily
}

/// The gains of the country's days `daily` holds, by currency.
fn gains(daily: &[LedgerDaily], mut held: [tally::Gain; 2]) -> io::Result<Vec<LedgerGain>> {
    for row in daily {
        for (index, gain) in held.iter_mut().enumerate() {
            gain.add(rows::gain(row, index == 1)?)?;
        }
    }
    Ok(["Political", "Command"]
        .into_iter()
        .zip(held)
        .filter(|(_, gain)| gain.any)
        .map(|(currency, gain)| LedgerGain {
            currency: currency.into(),
            gained_raw: gain.gained.to_string(),
            lost_raw: gain.lost.to_string(),
            complete: gain.incomplete == 0,
        })
        .collect())
}

/// Whether the census listed the country present, and refused, over a reply's hours: by the
/// hour each holds from, and the hours it changed at.
struct Presence {
    at: BTreeMap<i32, (bool, bool)>,
    changes: Vec<i32>,
}

/// The country's [`Presence`] over `from..=to`. Every tag is listed absent from the census's
/// first listing until it is listed otherwise.
fn presence<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    from: i32,
    to: i32,
) -> io::Result<Presence> {
    let mut presence = Presence {
        at: BTreeMap::new(),
        changes: Vec::new(),
    };
    let Some(listed) = reader
        .directory
        .scalar(emit::PRESENCE_FIRST)
        .and_then(Scalar::integer)
        .and_then(|hour| i32::try_from(hour).ok())
        .filter(|&listed| listed <= to)
    else {
        return Ok(presence);
    };
    let key = keys.key(reader.directory, PRESENT);
    if listed <= from {
        let state = match reader.latest(key, from)? {
            Some(event) => reader.record(event.fields, 0)?,
            None => (false, false),
        };
        presence.at.insert(from, state);
    } else {
        // The census's first listing opens a change.
        presence.at.insert(listed, (false, false));
        presence.changes.push(listed);
    }
    if from < to {
        for event in reader.events(key, from + 1, to)? {
            let state: (bool, bool) = reader.record(event.fields, 0)?;
            presence.at.insert(event.native_hour, state);
            if presence.changes.last() != Some(&event.native_hour) {
                presence.changes.push(event.native_hour);
            }
        }
    }
    Ok(presence)
}

/// The country's spending coverage over `from..=to`: where its census, presence and coverage runs
/// say its spending is recorded, why not elsewhere.
fn spending_coverage<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    presence: &Presence,
    from: i32,
    to: i32,
) -> io::Result<Vec<LedgerSpendCoverage>> {
    let directory = reader.directory;
    let branch = directory.actor(emit::BRANCH);
    let branch_key = |measure| branch.and_then(|actor| directory.key(actor, measure, None));
    let census = reader.runs(branch_key(emit::CENSUS), from, to)?;
    let mut boundaries = BTreeSet::from([i64::from(from), i64::from(to) + 1]);
    let mut presence_boundaries: BTreeSet<i64> = census
        .iter()
        .flat_map(|run| [i64::from(run.span.first), i64::from(run.span.last) + 1])
        .collect();
    presence_boundaries.extend(presence.changes.iter().map(|&hour| i64::from(hour)));
    boundaries.extend(&presence_boundaries);
    let mut banks = Vec::new();
    for measure in COVERED {
        let runs = reader.runs(keys.key(directory, measure), from, to)?;
        boundaries.extend(runs.iter().flat_map(|run| {
            [
                i64::from(run.span.first.max(from)),
                i64::from(run.span.last.min(to)) + 1,
            ]
        }));
        banks.push(runs);
    }
    let reasons_key = keys.key(directory, COVER_REASONS);
    let mut reasons = BTreeMap::<i32, LedgerSpendCoverage>::new();
    if let Some(event) = reader.latest(reasons_key, from)? {
        reasons.insert(event.native_hour, reader.record(event.fields, 0)?);
    }
    if from < to {
        for event in reader.events(reasons_key, from.saturating_add(1), to)? {
            boundaries.insert(i64::from(event.native_hour));
            reasons.insert(event.native_hour, reader.record(event.fields, 0)?);
        }
    }
    let boundaries: Vec<_> = boundaries.into_iter().collect();
    let mut positions = [0_usize; 2];
    let mut rows = Vec::new();
    for pair in boundaries.windows(2) {
        let first = pair[0] as i32;
        let last = (pair[1] - 1) as i32;
        if first > last {
            continue;
        }
        let supplied = census
            .iter()
            .any(|run| run.span.first <= first && first <= run.span.last);
        let at = presence
            .at
            .range(..=first)
            .next_back()
            .map(|(_, value)| *value);
        let present = supplied && at.is_some_and(|(present, _)| present);
        let absent = supplied && at.is_some_and(|(present, refused)| !present && !refused);
        let mut known = [false; 2];
        for (index, runs) in banks.iter().enumerate() {
            while positions[index] < runs.len() && runs[positions[index]].span.last < first {
                positions[index] += 1;
            }
            known[index] = runs
                .get(positions[index])
                .is_some_and(|run| run.span.first <= first && run.span.last >= first);
            known[index] |= absent;
        }
        let fact = reasons
            .range(..=first)
            .next_back()
            .map(|(_, fact)| fact)
            .filter(|fact| present && fact.political == known[0] && fact.command == known[1]);
        let value = LedgerSpendCoverage {
            // An unchanged reason can cross a gap; its supplying read belongs only to the
            // interval its fact recorded.
            read_hour: fact
                .filter(|fact| {
                    fact.since_hour
                        .zip(fact.through_hour)
                        .is_some_and(|(since, through)| since <= first && first <= through)
                })
                .map_or(first, |fact| fact.read_hour),
            since_hour: Some(first),
            through_hour: Some(last),
            political: known[0],
            command: known[1],
            reason: fact.and_then(|fact| fact.reason.clone()).or_else(|| {
                (!(known[0] && known[1])).then(|| "politics coverage unavailable".into())
            }),
        };
        if presence_boundaries.contains(&i64::from(first)) {
            rows.push(value);
        } else {
            rows::merge_spending_coverage(&mut rows, value);
        }
    }
    Ok(rows)
}

/// The country's command power holds over `from..=to`, each snapshot carried over accepted hours.
fn holds<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    presence: &Presence,
    from: i32,
    to: i32,
) -> io::Result<Vec<LedgerCommandHold>> {
    let directory = reader.directory;
    let key = keys.key(directory, HOLDS);
    let mut events = Vec::new();
    if let Some(event) = reader.latest(key, from)? {
        events.push(event);
    }
    if from < to {
        events.extend(reader.events(key, from + 1, to)?);
    }
    let branch = directory.actor(emit::BRANCH);
    let branch_key = |measure| branch.and_then(|actor| directory.key(actor, measure, None));
    let census = reader.runs(branch_key(emit::CENSUS), i32::MIN, to)?;
    let mut rows: Vec<LedgerCommandHold> = Vec::new();
    for event in events {
        let hour = event.native_hour;
        let mut row: LedgerCommandHold = reader.record(event.fields, 0)?;
        // A snapshot of a present country runs to the reply's last hour; the next closes it.
        let at = hour.max(from);
        let present = census
            .iter()
            .any(|run| run.span.first <= at && at <= run.span.last)
            && presence
                .at
                .range(..=at)
                .next_back()
                .is_some_and(|(_, (present, _))| *present);
        if present {
            row.through_hour = to;
        }
        if let Some(previous) = rows.last_mut() {
            if previous.read_hour == row.read_hour {
                previous.through_hour = row.through_hour;
                continue;
            }
            previous.through_hour = previous.through_hour.min(row.read_hour - 1);
        }
        rows.push(row);
    }
    // A hold carries over accepted hours, never the missing interval after its episode.
    if let Some(first) = rows.first().map(|row| row.read_hour) {
        let accepted = reader.runs(branch_key(emit::ACCEPTED), first, to)?;
        for row in &mut rows {
            let index = accepted.partition_point(|run| run.span.last < row.read_hour);
            if let Some(run) = accepted
                .get(index)
                .filter(|run| run.span.first <= row.read_hour)
            {
                row.through_hour = row.through_hour.min(run.span.last);
            }
        }
    }
    Ok(rows)
}

/// The reply's warnings: the branch's and the country's, and, but for governments, what
/// presenting its payments found.
fn warnings<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    presentation: Option<&Presentation>,
) -> io::Result<Vec<String>> {
    let mut reasons = reader.scalar_reasons(emit::WARNINGS);
    if let Some(actor) = keys.actor {
        for (reason, count) in reader.scalar_reasons(emit::TAG_WARNINGS + actor) {
            rows::count_reason(&mut reasons, reason, count)?;
        }
    }
    if let Some(presentation) = presentation {
        for (reason, count) in &presentation.reasons {
            rows::count_reason(&mut reasons, reason.clone(), *count)?;
        }
        for reason in &presentation.formats {
            rows::count_reason(&mut reasons, reason.clone(), 1)?;
        }
    }
    Ok(reasons
        .into_iter()
        .map(|(reason, count)| format!("{reason} (count: {count})"))
        .collect())
}

/// The country's governments native to `spans` and read by `to`, in read order.
fn governments<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    first: i32,
    to: i32,
    spans: &[HourSpan],
) -> io::Result<Vec<LedgerGovernment>> {
    let key = keys.key(reader.directory, GOVERNMENTS);
    let mut rows = Vec::new();
    let last = spans.last().map_or(to, |span| span.last_hour);
    for event in reader.events(key, first, last)? {
        if event.read_hour.is_some_and(|read| read > to) || !selected(spans, event.native_hour) {
            continue;
        }
        rows.push(reader.record::<LedgerGovernment>(event.fields, 0)?);
    }
    rows.sort_by_key(|row| (row.read_hour, row.country, row.sequence.parse::<u64>().ok()));
    Ok(rows)
}

/// The reply to a ledger request, from the branch's publication.
pub fn query<P: PageRead>(
    params: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params.clone())?;
    validate_tag(&params.tag).map_err(dashboard_core::invalid)?;
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid politics ledger branch"))?;
    let first = directory
        .scalar(emit::FIRST)
        .and_then(Scalar::integer)
        .and_then(|hour| i32::try_from(hour).ok());
    let accepted = snapshot.publication().accepted;
    let first = first.unwrap_or(accepted.first);
    let last = accepted.last;
    let settled = directory
        .scalar(emit::SETTLED)
        .and_then(Scalar::integer)
        .and_then(|hour| i32::try_from(hour).ok());
    let mut reader = Reader {
        snapshot,
        directory,
        first,
        last,
        settled,
        scratch: Vec::new(),
    };
    let keys = Keys::of(directory, &params.tag);
    if let Some(navigation) = &params.navigation
        && params.view != LedgerView::Governments
    {
        let view = params
            .summary
            .as_ref()
            .ok_or_else(|| invalid("Politics ledger navigation steps through a summary's parts"))?;
        return navigate(&mut reader, &params, view, navigation);
    }
    if let Some(view) = &params.summary {
        return summaries(&mut reader, &params, view);
    }
    let to = params.hour;
    let from = params.from.unwrap_or(reader.first);
    if from > to {
        return Err(invalid("Politics ledger range is reversed"));
    }
    let within = params.within.clone().unwrap_or_else(|| {
        vec![HourSpan {
            first_hour: from,
            last_hour: to,
        }]
    });
    let spans = normalize_spans(&within, from, to);
    if params.view == LedgerView::Governments {
        if let Some(navigation) = &params.navigation {
            return government_navigation(&mut reader, &keys, navigation, to);
        }
        if let Some(page) = &params.page {
            return government_page(&mut reader, &keys, &params.tag, page, to);
        }
        let mut reply = PoliticsLedgerReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: 5,
            ..Default::default()
        };
        if let Some(view) = &params.marks {
            series::viewport(Some(view.from), Some(view.to), Some(view.width))?;
            let window = normalize_spans(
                &[HourSpan {
                    first_hour: view.from.ceil() as i32,
                    last_hour: view.to.floor() as i32,
                }],
                i32::MIN,
                to,
            );
            reply.governments =
                governments(&mut reader, &keys, view.from.ceil() as i32, to, &window)?;
            reply.government_marks = Some(government_marks(&mut reader, &keys, view, to)?);
            reply.warnings = warnings(&mut reader, &keys, None)?;
            return Ok(serde_json::to_value(reply)?);
        }
        reply.governments = governments(&mut reader, &keys, from, to, &spans)?;
        if params.matches {
            // Rows supplied within the viewport `within` names, never past the reached read.
            let end = spans.last().map_or(to, |span| span.last_hour);
            let key = keys.key(directory, GOVERNMENTS);
            for event in reader.events(key, i32::MIN, end)? {
                if event
                    .read_hour
                    .is_none_or(|read| read < from || read > end)
                {
                    continue;
                }
                let row: LedgerGovernment = reader.record(event.fields, 0)?;
                if !reply.governments.iter().any(|old| {
                    (old.country, &old.epoch, &old.sequence)
                        == (row.country, &row.epoch, &row.sequence)
                }) {
                    reply.governments.push(row);
                }
            }
            reply
                .governments
                .sort_by_key(|row| (row.read_hour, row.country, row.sequence.parse::<u64>().ok()));
        }
        reply.warnings = warnings(&mut reader, &keys, None)?;
        if json_len(&reply, MAX_REPLY_BYTES).is_none() {
            return Err(invalid(
                "Government history exceeds the complete-response budget; no history was truncated",
            ));
        }
        return Ok(serde_json::to_value(reply)?);
    }
    // Totals read each settled period's rollup; everything else reads its own hours alone: those
    // `within` names, so nothing it excludes is read or presented.
    let rolled = params.totals && params.view == LedgerView::Spending;
    let plan = if rolled {
        plan(&mut reader, &keys, &spans, to)?
    } else {
        Plan::spans(&spans)
    };
    let mut country = Country::read(&mut reader, &params.tag, &plan.raw)?;
    let mut presentation = country.present(&reader, cancelled)?;
    let mut reply = PoliticsLedgerReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: 5,
        ..Default::default()
    };
    let view = params.view.clone();
    if view == LedgerView::Balance {
        for (read, _, record) in &country.power {
            if *read < from || *read > to || !selected(&spans, record.hour) {
                continue;
            }
            reply.balance_changes.push(LedgerBalanceChange {
                country_id: record.country,
                hour: record.hour,
                read_hour: *read,
                currency: currency(record.command).into(),
            });
        }
    }
    let daily = latest_daily(&country.daily, &spans, to);
    let listed = presence(&mut reader, &keys, from, to)?;
    reply.spending_coverage = spending_coverage(&mut reader, &keys, &listed, from, to)?;
    if view == LedgerView::Balance {
        reply.command_holds = holds(&mut reader, &keys, &listed, from, to)?;
    }
    let mut appended = Appended::default();
    let held = if rolled {
        totals(
            &mut reply,
            &mut appended,
            &mut reader,
            &mut country,
            &mut presentation,
            &plan,
            from,
            to,
            &spans,
            cancelled,
        )?
    } else {
        let presented = std::mem::take(&mut presentation.payments);
        payments(
            &mut reply,
            &mut appended,
            presented,
            reader.settled,
            &view,
            from,
            to,
            &spans,
            cancelled,
        )?;
        Default::default()
    };
    reply.gains = gains(&daily, held)?;
    reply.warnings = warnings(&mut reader, &keys, Some(&presentation))?;
    if view == LedgerView::Balance {
        reply.daily = daily;
    }
    reply.spend_categories.sort_by(|a, b| a.id.cmp(&b.id));
    reply.spending.sort_by_key(|row| {
        (
            row.hour,
            row.sequence.parse::<u64>().ok(),
            row.ordinal,
            row.id.clone(),
        )
    });
    if json_len(&reply, MAX_REPLY_BYTES).is_none() {
        return Err(invalid(
            "Prepared politics ledger exceeds the complete-response budget; no rows were truncated",
        ));
    }
    Ok(serde_json::to_value(reply)?)
}

/// The native days a span cuts: a span's partial first and last day.
fn boundary_days(spans: &[HourSpan]) -> BTreeSet<i32> {
    spans
        .iter()
        .flat_map(|span| {
            let mut days = Vec::new();
            if span.first_hour.rem_euclid(24) != 0 {
                days.push(last_day(span.first_hour) as i32);
            }
            if span.last_hour.rem_euclid(24) != 23 {
                days.push(last_day(span.last_hour) as i32);
            }
            days
        })
        .collect()
}

#[cfg(test)]
thread_local! {
    /// Whether this thread's totals read rollups, and how many they read.
    pub(super) static ROLLUPS: std::cell::Cell<(bool, usize)> =
        const { std::cell::Cell::new((true, 0)) };
}

/// The hours a reply reads: periods a rollup holds whole (first hour, last hour, totals), and
/// the hours it reads row by row.
struct Plan {
    rollups: Vec<(i32, i32, tally::Totals)>,
    raw: Vec<(i32, i32)>,
}

impl Plan {
    /// Raw reads of the native hours of `spans` alone.
    fn spans(spans: &[HourSpan]) -> Self {
        let mut plan = Self {
            rollups: Vec::new(),
            raw: Vec::new(),
        };
        for span in spans {
            plan.push_raw(span.first_hour, span.last_hour);
        }
        plan
    }

    fn push_raw(&mut self, first: i32, last: i32) {
        if first > last {
            return;
        }
        match self.raw.last_mut() {
            Some(held) if held.1.checked_add(1) == Some(first) => held.1 = last,
            _ => self.raw.push((first, last)),
        }
    }
}

/// `key`'s rollup of the `level` period ending at `hour`, when every row it counts is read by
/// `to`.
fn rollup<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    key: Option<u64>,
    level: usize,
    hour: i32,
    to: i32,
) -> io::Result<Option<tally::Totals>> {
    let kind = emit::ROLLUP_KIND + level as u64;
    let Some(event) = reader
        .events(key, hour, hour)?
        .into_iter()
        .filter(|event| event.kind == kind)
        .max_by_key(|event| event.sequence)
    else {
        return Ok(None);
    };
    if event.read_hour.is_none_or(|read| read > to) {
        return Ok(None);
    }
    let cells = reader.cells(event.fields)?;
    tally::read(&cells.0).map(Some)
}

/// The hours of `spans` read by `to` a totals reply reads: the largest settled periods a rollup
/// holds, then row by row what none holds.
fn plan<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    spans: &[HourSpan],
    to: i32,
) -> io::Result<Plan> {
    let key = keys.key(reader.directory, ROLLUP);
    let mut plan = Plan {
        rollups: Vec::new(),
        raw: Vec::new(),
    };
    #[cfg(test)]
    if !ROLLUPS.with(std::cell::Cell::get).0 {
        for span in spans {
            plan.push_raw(span.first_hour, span.last_hour.min(to));
        }
        return Ok(plan);
    }
    for span in spans {
        let (first, last) = (span.first_hour, span.last_hour.min(to));
        if first > last {
            continue;
        }
        let mut day = first.div_euclid(24) + i32::from(first.rem_euclid(24) != 0);
        let end = (last + 1).div_euclid(24) - 1;
        if key.is_none() || day > end {
            plan.push_raw(first, last);
            continue;
        }
        plan.push_raw(first, day * 24 - 1);
        while day <= end {
            let mut taken = None;
            for level in (0..tally::PERIOD_DAYS.len()).rev() {
                let size = tally::PERIOD_DAYS[level];
                if day.rem_euclid(size) != 0 || day + size - 1 > end {
                    continue;
                }
                let hour = (day + size) * 24 - 1;
                if let Some(totals) = rollup(reader, key, level, hour, to)? {
                    taken = Some((size, hour, totals));
                    break;
                }
            }
            match taken {
                Some((size, hour, totals)) => {
                    #[cfg(test)]
                    ROLLUPS.with(|cell| cell.set((cell.get().0, cell.get().1 + 1)));
                    plan.rollups.push((day * 24, hour, totals));
                    day += size;
                }
                None => {
                    plan.push_raw(day * 24, day * 24 + 23);
                    day += 1;
                }
            }
        }
        plan.push_raw((end + 1) * 24, last);
    }
    Ok(plan)
}

/// Spending totals by item over `spans` read by `to` ([`tally`]): the rollups `plan` read, the
/// hours it reads row by row folded the same way, and the late rows no rollup holds; the gains
/// of the same days.
#[allow(clippy::too_many_arguments)]
fn totals<P: PageRead>(
    reply: &mut PoliticsLedgerReply,
    appended: &mut Appended,
    reader: &mut Reader<'_, '_, P>,
    country: &mut Country,
    presentation: &mut Presentation,
    plan: &Plan,
    from: i32,
    to: i32,
    spans: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<[tally::Gain; 2]> {
    let boundary = boundary_days(spans);
    let mut entries: HashMap<Field, tally::Entry> = HashMap::new();
    let mut rolled: HashMap<Field, tally::Entry> = HashMap::new();
    let mut gains = [tally::Gain::default(); 2];
    let mut unmatched = 0_u64;
    let mut late_rows = Vec::new();
    let late = country.keys.key(reader.directory, LATE);
    let daily_key = country.keys.key(reader.directory, DAILY);
    for (first, last, held) in &plan.rollups {
        cancelled_request(cancelled)?;
        for (field, entry) in &held.entries {
            entries.entry(*field).or_default().merge(entry)?;
            rolled.entry(*field).or_default().merge(entry)?;
        }
        unmatched += held.unmatched;
        // Late rows of these hours, which no rollup holds; a daily row revised once its day
        // settled leaves the rollup's gains stale, so they are read row by row.
        let mut stale = false;
        for event in reader.events(late, *first, *last)? {
            if event.read_hour.is_some_and(|read| read > to) {
                continue;
            }
            match event.kind {
                emit::POWER_KIND => late_rows.push(Late::Power((
                    read_of(&event),
                    event.sequence,
                    reader.record(event.fields, 0)?,
                ))),
                emit::COST_KIND => late_rows.push(Late::Cost((
                    read_of(&event),
                    event.sequence,
                    reader.record(event.fields, 0)?,
                ))),
                _ => stale = true,
            }
        }
        if stale {
            let mut daily = Vec::new();
            for event in reader.events(daily_key, *first, *last)? {
                daily.push(reader.record::<DailyRecord>(event.fields, 0)?);
            }
            daily.sort_by_key(|row| (row.daily.day_hour, row.daily.read_hour));
            for row in latest_daily(&daily, spans, to) {
                for (index, gain) in gains.iter_mut().enumerate() {
                    gain.add(rows::gain(&row, index == 1)?)?;
                }
            }
        } else {
            for (gain, more) in gains.iter_mut().zip(&held.gains) {
                gain.merge(more)?;
            }
        }
    }
    let wanted: BTreeSet<u32> = late_rows
        .iter()
        .filter_map(|row| match row {
            Late::Power((_, _, record)) => (record.class != emit::SKIPPED).then_some(record.item),
            Late::Cost((_, _, record)) => shows_item(record).then_some(record.item),
        })
        .collect();
    country.load(reader, &wanted)?;
    let mut extra = Vec::new();
    for row in &late_rows {
        let presented = match row {
            Late::Power(row) => country.present_power(row, presentation)?,
            Late::Cost(row) => country.present_cost(row, presentation)?,
        };
        extra.extend(presented);
    }
    let settled = reader.settled;
    let main = std::mem::take(&mut presentation.payments);
    let mut templates: HashMap<Field, Payment> = HashMap::new();
    let mut days: BTreeMap<(i32, Field), (tally::Day, Option<Payment>)> = BTreeMap::new();
    let mut open = BTreeMap::<(i32, String), Payment>::new();
    let mut individual = Vec::new();
    for presented in main.iter().chain(&extra) {
        cancelled_request(cancelled)?;
        let payment = &presented.payment;
        let contribution = &presented.contribution;
        templates
            .entry(contribution.field)
            .or_insert_with(|| payment.clone());
        if !presented.recurring {
            if payment.spend.read_hour <= to && selected(spans, payment.spend.hour) {
                entries
                    .entry(contribution.field)
                    .or_default()
                    .one_off
                    .get_or_insert_with(tally::OneOff::default)
                    .add(contribution)?;
            }
            continue;
        }
        let day = payment.spend.hour.div_euclid(24) * 24;
        if presented.late {
            if payment.spend.read_hour <= to && selected(spans, payment.spend.hour) {
                individual.push(payment.clone());
            }
            continue;
        }
        if !settled.is_some_and(|settled| day < settled) {
            match open.entry((day, item_field(payment))) {
                std::collections::btree_map::Entry::Occupied(mut held) => {
                    merge(held.get_mut(), payment)?;
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(payment.clone());
                }
            }
            continue;
        }
        match days.get_mut(&(day, contribution.field)) {
            Some((held, merged)) => {
                held.add(contribution)?;
                if let Some(merged) = merged {
                    merge(merged, payment)?;
                }
            }
            None => {
                // Only a command day a span cuts is shown by its payments.
                let merged = (payment.spend.currency == "Command").then(|| payment.clone());
                days.insert(
                    (day, contribution.field),
                    (tally::Day::new(contribution), merged),
                );
            }
        }
    }
    for ((day, field), (held, merged)) in &days {
        if let Some(merged) = merged
            && boundary.contains(day)
        {
            if let Some(clipped) = clip_command_payment(merged.clone(), spans, to)? {
                individual.push(clipped);
            }
            continue;
        }
        if selected(spans, *day) && i64::from(*day) + i64::from(held.lag) <= i64::from(to) {
            entries.entry(*field).or_default().add_day(held)?;
        }
    }
    for ((day, _), payment) in open {
        let payment = if payment.spend.currency == "Command" && boundary.contains(&day) {
            match clip_command_payment(payment, spans, to)? {
                Some(clipped) => clipped,
                None => continue,
            }
        } else if payment.spend.read_hour <= to && selected(spans, payment.spend.hour) {
            payment
        } else {
            continue;
        };
        individual.push(payment);
    }
    // Items only rollups hold are named by their catalogue.
    let missing: BTreeSet<u32> = entries
        .keys()
        .filter(|field| !templates.contains_key(*field))
        .map(|field| field.0)
        .collect();
    country.load(reader, &missing)?;
    for &ordinal in &missing {
        presentation.show(ordinal, country.item(ordinal)?);
    }
    let mut fields: Vec<Field> = entries.keys().copied().collect();
    fields.sort_unstable();
    let count = |value: u64| u32::try_from(value).map_err(|_| count_overflow());
    for field in fields {
        let entry = entries[&field];
        let template = match templates.get(&field) {
            Some(template) => template.clone(),
            None => country.template(field)?,
        };
        // What rollups count of a line a reply shows says why its item is not named.
        if let Some(rolled) = rolled.get(&field) {
            let paid = rolled.one_off.map_or(0, |one| one.occurrences)
                + rolled
                    .recurring
                    .map_or(0, |recurring| recurring.occurrences);
            let unknown = country
                .items
                .get(&field.0)
                .and_then(|shown| shown.stored.item.unknown.clone());
            if let Some(reason) = unknown
                && paid + rolled.balances > 0
            {
                rows::count_reason(&mut presentation.reasons, reason, paid + rolled.balances)?;
            }
            if rolled.credits > 0 {
                let name = template
                    .paid
                    .item
                    .name
                    .as_deref()
                    .unwrap_or("Not identified");
                rows::count_reason(
                    &mut presentation.reasons,
                    format!(
                        "Negative daily political power cost component {name}: recorded credit is not included in spending"
                    ),
                    rolled.credits,
                )?;
            }
        }
        let name = format!("{}:{}:{}", field.0, field.1, field.2);
        if let Some(one) = entry.one_off {
            let mut payment = template.clone();
            payment.spend.id = format!("total:{}:{name}", reply.tag);
            payment.spend.hour = one.latest.0;
            payment.spend.sequence = one.latest.1.to_string();
            payment.spend.ordinal = one.latest.2;
            payment.spend.read_hour = one.read;
            payment.spend.charge_raw = (one.unknown == 0).then(|| one.known.to_string());
            payment.spend.realized_delta_raw = None;
            payment.spend.occurrences = count(one.occurrences)?;
            payment.spend.covered = one.uncovered == 0;
            payment.known = one.known;
            payment.known_count = count(one.occurrences - one.unknown)?;
            payment.count = count(one.entries)?;
            append_payment(
                reply,
                appended,
                alone(payment),
                Some((from, one.latest.0, 0)),
            )?;
        }
        if let Some(recurring) = entry.recurring {
            let mut payment = template;
            let first_hour = month_window_start(spans, recurring.first_day);
            let native_hour = recurring.last.day;
            payment.spend.id = format!("recurring:{}:{name}:None:{first_hour}", reply.tag);
            payment.spend.hour = recurring.last.hour;
            payment.spend.read_hour = native_hour.saturating_add(recurring.last.lag);
            payment.spend.sequence = recurring.last.sequence.to_string();
            payment.spend.ordinal = recurring.last.ordinal;
            payment.spend.charge_raw =
                (recurring.unknown == 0).then(|| recurring.charge.to_string());
            payment.known = recurring.charge;
            payment.known_count = count(recurring.occurrences - recurring.unknown)?;
            payment.spend.realized_delta_raw = (!recurring.realized_unknown)
                .then(|| (recurring.realized.0 + recurring.realized.1).to_string());
            payment.spend.occurrences = count(recurring.occurrences)?;
            payment.spend.covered = recurring.uncovered == 0;
            payment.count = count(recurring.entries)?;
            append_payment(
                reply,
                appended,
                alone(payment),
                Some((first_hour, native_hour, count(recurring.days)?)),
            )?;
        }
    }
    for payment in individual {
        let day = payment.spend.hour.div_euclid(24) * 24;
        append_payment(reply, appended, alone(payment), Some((day, day, 1)))?;
    }
    if unmatched > 0 {
        rows::count_reason(
            &mut presentation.reasons,
            "Daily political power cost: matched PP application is unavailable".into(),
            unmatched,
        )?;
    }
    Ok(gains)
}

/// The next or previous change of the summary's parts over the bound countries' spans.
fn navigate<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    params: &Params,
    view: &PartView,
    navigation: &Navigation,
) -> io::Result<Value> {
    let directory = reader.directory;
    let tags: Vec<&String> = params
        .bindings
        .as_ref()
        .map(|bindings| bindings.keys().collect())
        .unwrap_or_else(|| vec![&params.tag]);
    let currency = usize::from(view.currency == "Command");
    let mut selected = Vec::new();
    for tag in tags {
        let keys = Keys::of(directory, tag);
        let within = match &params.bindings {
            Some(bindings) => bindings.get(tag).map(Vec::as_slice),
            None => params.within.as_deref(),
        };
        let mut found = Vec::new();
        if view.mode == "income" {
            let dimensions: &[u64] = if currency == 0 {
                &[0, 1, 2, 3]
            } else {
                &[4, 5]
            };
            found.extend(
                dimensions
                    .iter()
                    .filter_map(|&dimension| keys.part(directory, dimension)),
            );
        } else {
            found.extend(keys.part(directory, emit::TOTAL[currency]));
            for (dimension, _) in categories(directory, &keys) {
                if category_currency(directory, dimension) == Some(currency) {
                    found.extend(keys.part(directory, dimension));
                }
            }
        }
        selected.extend(found.into_iter().map(|key| (key, within)));
    }
    let hour = engine::navigate(reader.snapshot, &selected, navigation)?;
    Ok(serde_json::json!({ "hour": hour }))
}

/// Each category part `keys`' country spent in, with its first hour.
fn categories(directory: &Directory, keys: &Keys) -> Vec<(u64, i32)> {
    let Some(actor) = keys.actor else {
        return Vec::new();
    };
    directory
        .scalar(emit::TAG_CATEGORIES + actor)
        .map(|scalar| {
            scalar
                .list()
                .iter()
                .filter_map(|pair| match pair.list() {
                    [dimension, first] => Some((
                        u64::try_from(dimension.integer()?).ok()?,
                        i32::try_from(first.integer()?).ok()?,
                    )),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The spelling of category part `dimension` (`Currency:category`).
fn category_name(directory: &Directory, dimension: u64) -> Option<String> {
    directory
        .scalar(CATEGORY_NAMES + dimension)
        .and_then(Scalar::text)
        .map(|text| String::from_utf8_lossy(text).into_owned())
}

fn category_currency(directory: &Directory, dimension: u64) -> Option<usize> {
    category_name(directory, dimension).map(|name| usize::from(name.starts_with("Command:")))
}

/// The chart of each part a power parts view asks: income or spending by category, each a
/// summary over the bound countries' spans.
fn summaries<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    params: &Params,
    view: &PartView,
) -> io::Result<Value> {
    series::viewport(Some(view.from), Some(view.to), Some(view.width))?;
    if !matches!(view.currency.as_str(), "Political" | "Command")
        || !matches!(view.mode.as_str(), "income" | "spending")
    {
        return Err(invalid("Invalid power parts selection"));
    }
    let directory = reader.directory;
    let (first, last) = (reader.first, reader.last);
    let currency = usize::from(view.currency == "Command");
    let mut reply = PoliticsLedgerReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: 5,
        ..Default::default()
    };
    let bindings = params.bindings.clone().unwrap_or_else(|| {
        BTreeMap::from([(
            params.tag.clone(),
            params.within.clone().unwrap_or_else(|| {
                vec![HourSpan {
                    first_hour: first,
                    last_hour: last,
                }]
            }),
        )])
    });
    // The parts asked: income parts fixed, or each category any bound country spent in.
    let mut fields: Vec<(String, String, Option<u64>)> = Vec::new();
    if view.mode == "income" {
        fields.push((
            "income".into(),
            "Daily gain".into(),
            Some(if currency == 0 { 0 } else { 4 }),
        ));
        if currency == 0 {
            fields.push(("costs".into(), "Daily costs".into(), Some(1)));
            fields.push(("wrapping".into(), "Adjustment".into(), Some(2)));
        }
        fields.push((
            "income-total".into(),
            "Total".into(),
            Some(if currency == 0 { 3 } else { 5 }),
        ));
    } else {
        let mut seen = BTreeSet::new();
        for tag in bindings.keys() {
            let keys = Keys::of(directory, tag);
            for (dimension, _) in categories(directory, &keys) {
                let Some(name) = category_name(directory, dimension) else {
                    continue;
                };
                let Some(id) = name.strip_prefix(&format!("{}:", view.currency)) else {
                    continue;
                };
                if seen.insert(id.to_owned()) {
                    fields.push((id.to_owned(), id.to_owned(), Some(dimension)));
                }
            }
        }
        fields.push(("spending-total".into(), "Total".into(), None));
    }
    for (id, label, dimension) in fields {
        let mut spans = Vec::new();
        for (tag, within) in &bindings {
            validate_tag(tag).map_err(dashboard_core::invalid)?;
            let keys = Keys::of(directory, tag);
            let dimension = match dimension {
                Some(dimension) if view.mode == "spending" => categories(directory, &keys)
                    .into_iter()
                    .map(|(found, _)| found)
                    .find(|found| {
                        category_name(directory, *found) == category_name(directory, dimension)
                    }),
                Some(dimension) => Some(dimension),
                None => Some(emit::TOTAL[currency]),
            };
            let actual = dimension.and_then(|dimension| keys.part(directory, dimension));
            let zero = (view.mode == "spending" && id != "spending-total")
                .then(|| keys.part(directory, emit::ZERO[currency]))
                .flatten();
            let origin = dimension.and_then(|dimension| {
                categories(directory, &keys)
                    .into_iter()
                    .find(|(found, _)| *found == dimension)
                    .map(|(_, first)| first)
            });
            for span in within {
                if let Some(zero) = zero {
                    let end = origin
                        .and_then(|hour| hour.checked_sub(1))
                        .unwrap_or(span.last_hour)
                        .min(span.last_hour);
                    if span.first_hour <= end {
                        spans.push((zero, span.first_hour, end));
                    }
                }
                if let Some(actual) = actual {
                    let start = if zero.is_some() {
                        origin.unwrap_or(span.first_hour).max(span.first_hour)
                    } else {
                        span.first_hour
                    };
                    if start <= span.last_hour {
                        spans.push((actual, start, span.last_hour));
                    }
                }
            }
        }
        spans.sort_by_key(|span| span.1);
        let summary = part_summary(
            reader,
            &spans,
            (first, last),
            (view.from, view.to, view.width),
        )?;
        let label = if view.mode == "spending" && id != "spending-total" {
            category_label(&id, &label)
        } else {
            label
        };
        reply
            .part_summaries
            .push(LedgerPartSummary { id, label, summary });
    }
    Ok(serde_json::to_value(reply)?)
}

/// A category's label: the native cause's name, an idea category's authored key.
fn category_label(id: &str, fallback: &str) -> String {
    for cause in (0..=40_u16).filter_map(|value| PowerCause::try_from(value).ok()) {
        let (known, name) = rows::category(cause);
        if known == id {
            return name.to_owned();
        }
    }
    match id {
        "dailyCosts" => "Daily costs".into(),
        "organisationUpkeep" => "Organisation upkeep".into(),
        "laws" => "Laws".into(),
        _ => fallback.to_owned(),
    }
}

/// The hours of each bin of a chart of `view` laid from `bounds`, as the series engine lays
/// them (`dashboard_series::query::Grid`).
fn bins(bounds: (i32, i32), view: (f64, f64, u32)) -> Vec<(i64, i64)> {
    let (from, to, width) = view;
    let first = from.floor().max(f64::from(bounds.0));
    let last = to.floor().min(f64::from(bounds.1));
    if width == 0 || first > last {
        return Vec::new();
    }
    let (first, last) = (first as i64, last as i64);
    let count = last - first + 1;
    let per_pixel = (count / i64::from(width)).max(1);
    let fine = per_pixel <= 16;
    let size = if fine {
        1_i64 << (63 - (per_pixel as u64).leading_zeros())
    } else {
        count.div_euclid(i64::from(width)) + i64::from(count % i64::from(width) != 0)
    };
    let base = if fine {
        i64::from(bounds.0) + (first - i64::from(bounds.0)) / size * size
    } else {
        first
    };
    (0..(last - base) / size + 1)
        .map(|index| {
            let start = base + index * size;
            (start.max(first), (start + size - 1).min(last))
        })
        .collect()
}

/// One chart of the runs of `spans` (key, first, last), pieces of one series in hour order,
/// binned over `view`: each bin merges, in order, the pieces its hours meet, so a piece with no
/// value there leaves its break. Pieces that share an hour are refused.
fn part_summary<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    spans: &[(u64, i32, i32)],
    bounds: (i32, i32),
    view: (f64, f64, u32),
) -> io::Result<dashboard_base::prepared_chart::ChartSummary> {
    if spans.windows(2).any(|pair| pair[1].1 <= pair[0].2) {
        return Err(invalid(
            "Power parts: the bound countries hold some hours together; their parts are not one series",
        ));
    }
    let bins = bins(bounds, view);
    let mut buckets: Vec<Option<dashboard_base::prepared_chart::ChartBucket>> =
        vec![None; bins.len()];
    let mut met = vec![false; bins.len()];
    let mut runs: Option<Vec<dashboard_base::prepared_chart::ChartRun>> = None;
    let mut approximate = Vec::new();
    for &(key, first, last) in spans {
        let within = [HourSpan {
            first_hour: first,
            last_hour: last,
        }];
        let summary = engine::summary(
            reader.snapshot,
            key,
            bounds,
            view,
            emit::DECIMALS,
            Some(&within),
        )?;
        if summary.buckets.len() != bins.len() {
            return Err(invalid(
                "A power part's chart is not laid on its view's bins",
            ));
        }
        for (at, piece) in summary.buckets.into_iter().enumerate() {
            let (start, end) = bins[at];
            if end < i64::from(first) || start > i64::from(last) {
                continue;
            }
            buckets[at] = if met[at] {
                dashboard_base::prepared_chart::merge(buckets[at], piece)
            } else {
                piece
            };
            met[at] = true;
        }
        if let Some(more) = summary.runs {
            runs.get_or_insert_with(Vec::new).extend(more);
        }
        approximate.extend(summary.approximate);
    }
    if let Some(runs) = &mut runs {
        runs.sort_by_key(|run| run.first_hour);
    }
    Ok(dashboard_base::prepared_chart::ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets,
        runs,
        approximate,
    })
}

/// The next or previous government stop, among those read by `to`.
fn government_navigation<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    navigation: &Navigation,
    to: i32,
) -> io::Result<Value> {
    let origin = navigation.origin()?;
    let forward = matches!(navigation.key, MeasureKey::ArrowRight | MeasureKey::Home);
    let first = navigation.first_hour.ceil() as i32;
    let last = navigation.last_hour.floor() as i32;
    let key = keys.key(reader.directory, GOVERNMENTS);
    let mut result: Option<f64> = None;
    for event in reader.events(key, first, last)? {
        if event.read_hour.is_some_and(|read| read > to) {
            continue;
        }
        let candidate = f64::from(event.native_hour);
        if (forward && candidate > origin) || (!forward && candidate < origin) {
            result = Some(result.map_or(candidate, |old| {
                if forward {
                    old.min(candidate)
                } else {
                    old.max(candidate)
                }
            }));
        }
    }
    Ok(serde_json::json!({
        "hour": result.or_else(|| navigation.from.map(|hour| hour.clamp(navigation.first_hour, navigation.last_hour)))
    }))
}

/// A page of the country's governments read by `to`.
fn government_page<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    tag: &str,
    page: &GovernmentPage,
    to: i32,
) -> io::Result<Value> {
    let (from, end, cursor, size) = series::event_window(
        Some(page.from),
        Some(page.to),
        Some(page.cursor.as_str()),
        Some(page.page_size),
    )?;
    let end = end.min(to);
    let key = keys.key(reader.directory, GOVERNMENTS);
    let mut visible = Vec::new();
    for event in reader.events(key, from, end)? {
        if event.read_hour.is_some_and(|read| read > to) {
            continue;
        }
        visible.push(event);
    }
    let total = visible.len() as u64;
    let start = usize::try_from(cursor)
        .unwrap_or(usize::MAX)
        .min(visible.len());
    let mut rows = Vec::new();
    for (ordinal, event) in visible.iter().enumerate().skip(start).take(size as usize) {
        let row: LedgerGovernment = reader.record(event.fields, 0)?;
        rows.push(MeasureEvent {
            tag: tag.to_owned(),
            hour: row.hour,
            ordinal: ordinal.to_string(),
            row: serde_json::to_value(row)?,
        });
    }
    let next = start + rows.len();
    Ok(serde_json::to_value(MeasureDropReply {
        coverage: None,
        first_ordinal: start.to_string(),
        total: total.to_string(),
        rows,
        warnings: Vec::new(),
        cursor: (next < visible.len()).then(|| next.to_string()),
    })?)
}

/// Marks of the country's governments over the viewport: each engine cluster, less the
/// governments read after `to`.
fn government_marks<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    view: &MarkView,
    to: i32,
) -> io::Result<MarkSummary> {
    let Some(key) = keys.key(reader.directory, GOVERNMENTS) else {
        return Ok(MarkSummary {
            from: view.from,
            to: view.to,
            width: view.width,
            clusters: Vec::new(),
        });
    };
    let mut summary = engine::clusters(reader.snapshot, key, (view.from, view.to, view.width))?;
    let mut clusters = Vec::with_capacity(summary.clusters.len());
    for cluster in std::mem::take(&mut summary.clusters) {
        let events = reader.events(Some(key), cluster.first_hour, cluster.last_hour)?;
        let visible: Vec<&Event> = events
            .iter()
            .filter(|event| event.read_hour.is_none_or(|read| read <= to))
            .collect();
        if visible.len() == events.len() {
            clusters.push(cluster);
            continue;
        }
        if let (Some(first), Some(last)) = (visible.first(), visible.last()) {
            clusters.push(MarkCluster {
                first_hour: first.native_hour,
                last_hour: last.native_hour,
                first_ordinal: cluster.first_ordinal,
                count: visible.len().to_string(),
            });
        }
    }
    summary.clusters = clusters;
    Ok(summary)
}
