//! The ledger's replies, read from its series publication ([`super::emit`]): a country's
//! recorded rows of the asked hours alone, named and grouped when the reply is asked. A
//! payment's item is the one the emit interned for it, named by its first payment and keying
//! its tooltip; it is resolved from the facts its first payment's hour recorded and the
//! recorded content, once for every reply. Totals read each settled period's rollup whole
//! ([`super::tally`]) and only the hours no rollup holds row by row.

use super::emit::{
    self, CAPTURE, CAPTURE_FACTS, CATEGORY_NAMES, COSTS, COVER_REASONS, COVERED,
    DAILY, DATED, GOVERNMENTS, HOLDS, INCOME, ITEMS, LATE, PARTS, POWER, PRESENCE, ROLLUP,
};
use super::items::{self, Dated, FrameTexts, Item};
use super::present::{self, Resolved};
use super::tally::{self, Contribution, Field};
use super::*;
use dashboard_base::prepared::engine;
use dashboard_base::prepared::series::{self, MeasureKey, Navigation};
use dashboard_base::prepared_chart::{MarkCluster, MarkSummary};
use dashboard_base::prepared_measure::query::{MeasureDropReply, MeasureEvent};
use dashboard_base::products::country_groups::HourSpan;
use dashboard_base::products::idea_format;
use dashboard_base::products::idea_history::{self, IdeaInfo};
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_base::products::{MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES, validate_tag};
use dashboard_core::transport::json_len;
use dashboard_series::{
    Cell, Event, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span,
    query::EventView,
    codec::Binary,
    family::{Directory, Scalar},
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    #[serde(default)]
    view: LedgerView,
    from: Option<i32>,
    hour: Option<i32>,
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
}

/// An event's cells.
struct Cells(Vec<Cell>);

impl Cells {
    fn integer(&self, at: usize) -> Option<i128> {
        match self.0.get(at)? {
            Cell::Integer(value) => Some(*value),
            Cell::Unsigned(value) => Some(i128::from(*value)),
            _ => None,
        }
    }

    fn unsigned(&self, at: usize) -> Option<u64> {
        match self.0.get(at)? {
            Cell::Unsigned(value) => Some(*value),
            Cell::Integer(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    fn u32(&self, at: usize) -> Option<u32> {
        self.unsigned(at)
            .and_then(|value| u32::try_from(value).ok())
    }

    fn i32(&self, at: usize) -> Option<i32> {
        self.integer(at).and_then(|value| i32::try_from(value).ok())
    }

    fn i64(&self, at: usize) -> Option<i64> {
        self.integer(at).and_then(|value| i64::try_from(value).ok())
    }

    fn boolean(&self, at: usize) -> bool {
        matches!(self.0.get(at), Some(Cell::Boolean(true)))
    }

    fn required<T>(value: Option<T>) -> io::Result<T> {
        value.ok_or_else(|| invalid("A politics ledger event is malformed"))
    }
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

    fn bytes(&mut self, cell: Option<&Cell>) -> io::Result<Option<Vec<u8>>> {
        match cell {
            Some(Cell::Text(blob)) => {
                let length =
                    usize::try_from(blob.len).map_err(|_| invalid("A ledger text is too long"))?;
                let mut text = vec![0; length];
                if length != 0 {
                    self.snapshot.cell_bytes(*blob, 0, &mut text)?;
                }
                Ok(Some(text))
            }
            _ => Ok(None),
        }
    }

    fn text(&mut self, cell: Option<&Cell>) -> io::Result<Option<String>> {
        Ok(self
            .bytes(cell)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
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

    fn identity(&mut self, cell: Option<&Cell>) -> io::Result<PowerIdentity> {
        let Some(Cell::List(blob)) = cell else {
            return Err(invalid("A ledger item identity is malformed"));
        };
        let cells = self.cells(*blob)?;
        let kind =
            |value: Option<u64>| Cells::required(value.and_then(|value| u8::try_from(value).ok()));
        Ok(PowerIdentity {
            action: PowerAction::try_from(kind(cells.unsigned(0))?)?,
            item_token: cells.u32(1),
            item_key: self.bytes(cells.0.get(2))?.unwrap_or_default(),
            slot_key: self.bytes(cells.0.get(3))?.unwrap_or_default(),
            character: cells.u32(4),
            object_kind: PowerObjectKind::try_from(kind(cells.unsigned(5))?)?,
            object_reference: cells.unsigned(6),
            target_country: cells.u32(7),
            target_state: cells.u32(8),
            target_region: cells.u32(9),
            origin_kind: PowerOriginKind::try_from(kind(cells.unsigned(10))?)?,
            origin_token: cells.u32(11),
            origin_key: self.bytes(cells.0.get(12))?.unwrap_or_default(),
        })
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
}

/// A recorded power row.
#[derive(Clone)]
struct PowerEvent {
    read: i32,
    sequence: u64,
    row: CountryPowerChangeRow,
    covered: bool,
    dated: Option<(i32, u64)>,
    late: bool,
    classed: Classed,
}

/// What the emit made of a payment ([`emit::ITEM_CELLS`]): its item's ordinal and first
/// payment's id, its class, and whether it recurs and counts its days one entry each.
#[derive(Clone, Debug)]
struct Classed {
    ordinal: u32,
    first: String,
    class: u64,
    recurring: bool,
    upkeep: bool,
}

fn classed(cells: &Cells, at: usize) -> io::Result<Classed> {
    let country = Cells::required(cells.u32(at + 2))?;
    let epoch = Cells::required(cells.unsigned(at + 3))?;
    let first = if cells.boolean(at + 1) {
        rows::cost_id(
            country,
            epoch,
            Cells::required(cells.i32(at + 5))?,
            Cells::required(cells.u32(at + 6))?,
        )
    } else {
        rows::mutation_id(country, epoch, Cells::required(cells.unsigned(at + 4))?)
    };
    Ok(Classed {
        ordinal: Cells::required(cells.u32(at))?,
        first,
        class: Cells::required(cells.unsigned(at + 7))?,
        recurring: cells.boolean(at + 8),
        upkeep: cells.boolean(at + 9),
    })
}

/// A recorded cost or its revision.
#[derive(Clone)]
struct CostEvent {
    read: i32,
    sequence: u64,
    row: CountryPowerDailyCostRow,
    covered: bool,
    dated: Option<(i32, u64)>,
    revision: bool,
    late: bool,
    classed: Classed,
}

/// A daily row's revision.
#[derive(Clone)]
struct DailyEvent {
    read: i32,
    row: CountryPowerDailyRow,
    reads: (Option<i32>, Option<i32>),
}

fn decode_power<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    event: &Event,
) -> io::Result<PowerEvent> {
    let cells = reader.cells(event.fields)?;
    let identity = reader.identity(cells.0.get(13))?;
    let kind8 = |at: usize| {
        Cells::required(
            cells
                .unsigned(at)
                .and_then(|value| u8::try_from(value).ok()),
        )
    };
    let kind16 = |at: usize| {
        Cells::required(
            cells
                .unsigned(at)
                .and_then(|value| u16::try_from(value).ok()),
        )
    };
    Ok(PowerEvent {
        read: event.read_hour.unwrap_or(event.native_hour),
        sequence: event.sequence,
        row: CountryPowerChangeRow {
            country: Cells::required(cells.u32(0))?,
            instance: Cells::required(cells.u32(1))?,
            epoch: Cells::required(cells.unsigned(2))?,
            sequence: Cells::required(cells.unsigned(3))?,
            hour: Cells::required(cells.i32(4))?,
            currency: PowerCurrency::try_from(kind8(5)?)?,
            operation: PowerOperation::try_from(kind8(6)?)?,
            cause: PowerCause::try_from(kind16(7)?)?,
            site: PowerSite::try_from(kind16(8)?)?,
            requested_delta_raw: cells.i64(9),
            balance_before_raw: Cells::required(cells.i64(10))?,
            balance_after_raw: Cells::required(cells.i64(11))?,
            identity,
        },
        covered: cells.boolean(12),
        dated: cells.i32(14).zip(cells.unsigned(15)),
        late: cells.boolean(16),
        classed: classed(&cells, 17)?,
    })
}

fn decode_cost<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    event: &Event,
) -> io::Result<CostEvent> {
    let cells = reader.cells(event.fields)?;
    let identity = reader.identity(cells.0.get(9))?;
    let kind8 = |at: usize| {
        Cells::required(
            cells
                .unsigned(at)
                .and_then(|value| u8::try_from(value).ok()),
        )
    };
    let kind16 = |at: usize| {
        Cells::required(
            cells
                .unsigned(at)
                .and_then(|value| u16::try_from(value).ok()),
        )
    };
    Ok(CostEvent {
        read: event.read_hour.unwrap_or(event.native_hour),
        sequence: event.sequence,
        row: CountryPowerDailyCostRow {
            country: Cells::required(cells.u32(0))?,
            instance: Cells::required(cells.u32(1))?,
            epoch: Cells::required(cells.unsigned(2))?,
            ordinal: Cells::required(cells.u32(3))?,
            pp_sequence: Cells::required(cells.unsigned(4))?,
            component: PowerDailyCostKind::try_from(kind8(5)?)?,
            cost_raw: Cells::required(cells.i64(6))?,
            cause: PowerCause::try_from(kind16(7)?)?,
            site: PowerSite::try_from(kind16(8)?)?,
            day_hour: event.native_hour,
            identity,
        },
        covered: cells.boolean(10),
        dated: cells.i32(11).zip(cells.unsigned(12)),
        revision: cells.boolean(13),
        late: cells.boolean(14),
        classed: classed(&cells, 15)?,
    })
}

fn decode_daily<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    event: &Event,
) -> io::Result<DailyEvent> {
    let cells = reader.cells(event.fields)?;
    Ok(DailyEvent {
        read: event.read_hour.unwrap_or(event.native_hour),
        row: CountryPowerDailyRow {
            country: Cells::required(cells.u32(0))?,
            instance: Cells::required(cells.u32(1))?,
            epoch: Cells::required(cells.unsigned(2))?,
            day_hour: event.native_hour,
            pp_gross_gain_raw: cells.i64(3),
            pp_cost_raw: cells.i64(4),
            pp_net_gain_raw: cells.i64(5),
            pp_realized_delta_raw: cells.i64(6),
            pp_sequence: cells.unsigned(7),
            cp_gain_raw: cells.i64(8),
            cp_realized_delta_raw: cells.i64(9),
            cp_sequence: cells.unsigned(10),
        },
        reads: (cells.i32(11), cells.i32(12)),
    })
}

fn decode_government<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    event: &Event,
) -> io::Result<LedgerGovernment> {
    let cells = reader.cells(event.fields)?;
    let kind = |at: usize| {
        Cells::required(
            cells
                .unsigned(at)
                .and_then(|value| u8::try_from(value).ok()),
        )
    };
    let row = GovernmentChangeRow {
        country: Cells::required(cells.u32(0))?,
        instance: Cells::required(cells.u32(1))?,
        epoch: Cells::required(cells.unsigned(2))?,
        sequence: Cells::required(cells.unsigned(3))?,
        hour: event.native_hour,
        before_party: cells.u32(4),
        after_party: cells.u32(5),
        before_ideology: cells.u32(6),
        after_ideology: cells.u32(7),
        cause: GovernmentCause::try_from(kind(11)?)?,
        actor_country: cells.u32(10),
        origin_kind: PowerOriginKind::try_from(kind(12)?)?,
        origin_token: cells.u32(13),
        origin_key: reader.bytes(cells.0.get(14))?.unwrap_or_default(),
        site: PowerSite::default(),
    };
    let tokens = (reader.text(cells.0.get(8))?, reader.text(cells.0.get(9))?);
    Ok(rows::government(
        &row,
        event.read_hour.unwrap_or(event.native_hour),
        tokens,
        cells.boolean(15),
    ))
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

/// A payment, as preparation projected one and a reply groups them.
#[derive(Clone)]
struct Payment {
    spend: LedgerSpend,
    item: LedgerSpendItem,
    category: LedgerSpendCategory,
    ideas: Option<LedgerIdeaTooltips>,
    count: u32,
    known_raw: String,
    known_count: u32,
    identity: (u32, String),
    ordinal: u32,
}

/// The item a payment groups under, of the capture identity that recorded it.
fn item_field(payment: &Payment) -> io::Result<String> {
    Ok(format!(
        "{}:{}:{}",
        payment.ordinal, payment.identity.0, payment.identity.1
    ))
}

fn add_raw(left: &mut Option<String>, right: Option<&str>) -> io::Result<()> {
    *left = match (rows::amount(left.as_deref())?, rows::amount(right)?) {
        (Some(left), Some(right)) => Some(
            left.checked_add(right)
                .ok_or_else(|| invalid("Politics subtotal overflow"))?
                .to_string(),
        ),
        _ => None,
    };
    Ok(())
}

fn merge_payment(left: &mut Payment, right: Payment) -> io::Result<()> {
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
        .ok_or_else(|| invalid("Politics payment count overflow"))?;
    left.spend.covered &= right.spend.covered;
    if left.spend.category != "upkeep" {
        left.count = left
            .count
            .checked_add(right.count)
            .ok_or_else(|| invalid("Politics payment count overflow"))?;
    }
    left.known_count = left
        .known_count
        .checked_add(right.known_count)
        .ok_or_else(|| invalid("Politics payment count overflow"))?;
    left.known_raw = rows::amount(Some(&left.known_raw))?
        .unwrap_or(0)
        .checked_add(rows::amount(Some(&right.known_raw))?.unwrap_or(0))
        .ok_or_else(|| invalid("Politics subtotal overflow"))?
        .to_string();
    left.spend.read_hour = left.spend.read_hour.max(right.spend.read_hour);
    if right.spend.hour < left.spend.hour {
        left.spend.hour = right.spend.hour;
        left.spend.country_name = None;
    }
    // The open numeric subtotal has no completed-payment list.
    left.spend.changes.clear();
    Ok(())
}

/// [`merge_payment`], keeping both sides' contributions as changes.
fn merge_keeping(left: &mut Payment, right: &Payment) -> io::Result<()> {
    let mut changes = std::mem::take(&mut left.spend.changes);
    changes.extend(right.spend.changes.iter().cloned());
    merge_payment(left, right.clone())?;
    left.spend.changes = changes;
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

/// The whole native days of `spans` by game month, undated days first, each its own window.
fn month_spans(spans: &[HourSpan]) -> Vec<(Option<i32>, Vec<HourSpan>)> {
    let mut result = BTreeMap::<_, Vec<_>>::new();
    let mut undated = Vec::new();
    for span in spans {
        let mut day = first_day(span.first_hour);
        let last = last_day(span.last_hour);
        while day <= last {
            let (month, next) = month(day as i32);
            let end = last.min(next - 24);
            let window = HourSpan {
                first_hour: day as i32,
                last_hour: end as i32,
            };
            if month.is_some() {
                result.entry(month).or_default().push(window);
            } else {
                undated.push((None, vec![window]));
            }
            day = next;
        }
    }
    undated.extend(result);
    undated
}

/// A payment of the country, as its presentation made it, and what totals count of it.
#[derive(Clone)]
struct Presented {
    payment: Payment,
    contribution: Contribution,
    recurring: bool,
    late: bool,
}

/// The facts of one country's items, each read once.
struct Facts {
    held: HashMap<(i32, u64), Arc<Dated>>,
    read: BTreeSet<i32>,
}

/// The presentation of the country's payments in the hours a reply reads.
struct Presentation {
    payments: Vec<Presented>,
    /// Each mutation's balance line, by (country, epoch, sequence).
    balance: HashMap<(u32, u64, u64), (String, Option<String>)>,
    reasons: BTreeMap<String, u64>,
    formats: BTreeSet<String>,
}

/// An item as every reply shows it: its resolution, its tooltip and the warnings formatting it
/// gave.
struct Shown {
    resolved: Resolved,
    ideas: Option<LedgerIdeaTooltips>,
    formats: Vec<String>,
    /// Its country's name at its first payment.
    country_name: Option<String>,
    /// The content it was named with, held so that no other content takes its address.
    _content: Option<Arc<PoliticsContent>>,
}

/// An item of a publication's country under one content: the publication's identity (source,
/// content, branch, schema), the tag, the item's ordinal and the content's address.
type ShownKey = ([u8; 32], [u8; 32], [u8; 32], u64, Box<str>, u32, usize);

/// Items shown, kept between replies: an item's ordinal names the same item for as long as
/// its publication grows. Dropped whole past this many.
const SHOWN_ITEMS: usize = 1 << 18;
static SHOWN: LazyLock<Mutex<HashMap<ShownKey, Arc<Shown>>>> = LazyLock::new(Mutex::default);

struct Country<'c> {
    tag: String,
    keys: Keys,
    content: Option<&'c Arc<PoliticsContent>>,
    identity: ([u8; 32], [u8; 32], [u8; 32], u64),
    facts: Facts,
    shown: HashMap<u32, Arc<Shown>>,
    power: Vec<PowerEvent>,
    costs: Vec<CostEvent>,
    daily: Vec<DailyEvent>,
    /// Each day's political application read hours, by country and day: (read, its pp read).
    applications: HashMap<(u32, i32), Vec<(i32, Option<i32>)>>,
}

impl<'c> Country<'c> {
    /// The country's rows of native hours within `windows`.
    fn read<P: PageRead>(
        reader: &mut Reader<'_, '_, P>,
        tag: &str,
        content: Option<&'c Arc<PoliticsContent>>,
        windows: &[(i32, i32)],
    ) -> io::Result<Self> {
        let keys = Keys::of(reader.directory, tag);
        let (mut power, mut costs, mut daily) = (Vec::new(), Vec::new(), Vec::new());
        for &(first, last) in windows {
            for measure in POWER {
                let key = keys.key(reader.directory, measure);
                for event in reader.events(key, first, last)? {
                    power.push(decode_power(reader, &event)?);
                }
            }
            for event in reader.events(keys.key(reader.directory, COSTS), first, last)? {
                costs.push(decode_cost(reader, &event)?);
            }
            for event in reader.events(keys.key(reader.directory, DAILY), first, last)? {
                daily.push(decode_daily(reader, &event)?);
            }
        }
        power.sort_by_key(|event| (event.read, event.sequence));
        costs.sort_by_key(|event| (event.read, event.sequence));
        daily.sort_by_key(|event| (event.row.day_hour, event.read));
        let mut applications = HashMap::<(u32, i32), Vec<(i32, Option<i32>)>>::new();
        for day in &daily {
            applications
                .entry((day.row.country, day.row.day_hour))
                .or_default()
                .push((day.read, day.reads.0));
        }
        let identity = reader.snapshot.identity();
        Ok(Self {
            tag: tag.to_owned(),
            keys,
            content,
            identity: (
                identity.source,
                identity.content,
                identity.branch,
                identity.schema,
            ),
            facts: Facts {
                held: HashMap::new(),
                read: BTreeSet::new(),
            },
            shown: HashMap::new(),
            power,
            costs,
            daily,
            applications,
        })
    }

    fn content(&self) -> Option<&'c PoliticsContent> {
        self.content.map(|content| &**content)
    }

    /// The facts interned at `at`.
    fn dated<P: PageRead>(
        &mut self,
        reader: &mut Reader<'_, '_, P>,
        at: Option<(i32, u64)>,
    ) -> io::Result<Option<Arc<Dated>>> {
        let Some((hour, sequence)) = at else {
            return Ok(None);
        };
        if self.facts.read.insert(hour) {
            let key = self.keys.key(reader.directory, DATED);
            for event in reader.events(key, hour, hour)? {
                let cells = reader.cells(event.fields)?;
                let bytes = reader
                    .bytes(cells.0.first())?
                    .ok_or_else(|| invalid("Interned ledger facts are malformed"))?;
                let dated: Dated = serde_json::from_slice(&bytes)?;
                self.facts
                    .held
                    .insert((event.native_hour, event.sequence), Arc::new(dated));
            }
        }
        Ok(self.facts.held.get(&(hour, sequence)).cloned())
    }

    /// Item `ordinal`, first paid by `row` (of `cause`, with `component`), as replies show it:
    /// resolved once for every reply of its publication under this content.
    #[allow(clippy::too_many_arguments)]
    fn shown<P: PageRead>(
        &mut self,
        reader: &mut Reader<'_, '_, P>,
        ordinal: u32,
        row: &LedgerMutation,
        identity: &PowerIdentity,
        cause: PowerCause,
        dated: Option<(i32, u64)>,
        component: Option<PowerDailyCostKind>,
    ) -> io::Result<Arc<Shown>> {
        if let Some(found) = self.shown.get(&ordinal) {
            return Ok(Arc::clone(found));
        }
        let key: ShownKey = (
            self.identity.0,
            self.identity.1,
            self.identity.2,
            self.identity.3,
            Box::from(self.tag.as_str()),
            ordinal,
            self.content
                .map_or(0, |content| Arc::as_ptr(content) as usize),
        );
        let kept = SHOWN
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        let shown = match kept {
            Some(shown) => shown,
            None => {
                let component_name = component.map(|component| format!("{component:?}"));
                let recorded = self.dated(reader, dated)?;
                let facts = match &recorded {
                    Some(dated) => Arc::clone(dated),
                    None => Arc::new(frameless(identity, cause, component, self.content())),
                };
                let (mut item, mut info, category) = present::resolve(
                    &facts,
                    row,
                    cause,
                    self.content(),
                    String::new(),
                    component_name.as_deref(),
                );
                // The item's ordinal keys its tooltip in every reply.
                if item.tooltip_token.is_some() {
                    let token = format!("idea:{}:{ordinal}", self.tag);
                    item.tooltip_token = Some(token.clone());
                    if let Some(info) = &mut info {
                        info.token = token;
                    }
                }
                let mut formats = BTreeSet::new();
                let ideas = info
                    .clone()
                    .map(|info| self.tooltip(&item, info, recorded.as_deref(), &mut formats));
                let shown = Arc::new(Shown {
                    resolved: (item, info, category),
                    ideas,
                    formats: formats.into_iter().collect(),
                    country_name: recorded
                        .as_ref()
                        .and_then(|dated| dated.own.as_ref())
                        .and_then(|own| own.name.clone()),
                    _content: self.content.cloned(),
                });
                let mut kept = SHOWN.lock().unwrap_or_else(PoisonError::into_inner);
                if kept.len() >= SHOWN_ITEMS {
                    kept.clear();
                }
                kept.insert(key, Arc::clone(&shown));
                shown
            }
        };
        self.shown.insert(ordinal, Arc::clone(&shown));
        Ok(shown)
    }

    /// The modifiers a tooltip of `infos` formats, with the warnings formatting them gives.
    fn formats(
        &self,
        infos: &[IdeaInfo],
        recorded: &[(String, items::RecordedModifier)],
        any_recorded: bool,
    ) -> (Vec<idea_format::ModifierFormat>, Vec<String>) {
        let Some(content) = self.content() else {
            return (Vec::new(), Vec::new());
        };
        let (keys, _) = idea_history::effect_keys(infos);
        let index: BTreeMap<String, ModifierDefinitionRow> = recorded
            .iter()
            .map(|(key, row)| (key.clone(), row.row()))
            .collect();
        idea_format::modifier_formats_from_recorded(any_recorded.then_some(&index), &keys, content)
    }

    /// The idea tooltip an item shows: its idea, artwork and effect formats.
    fn tooltip(
        &self,
        item: &LedgerSpendItem,
        info: IdeaInfo,
        dated: Option<&Dated>,
        formats: &mut BTreeSet<String>,
    ) -> LedgerIdeaTooltips {
        let infos = [info];
        let recorded = dated.and_then(|dated| dated.modifiers.as_deref());
        let (modifiers, warnings) =
            self.formats(&infos, recorded.unwrap_or_default(), recorded.is_some());
        formats.extend(warnings);
        let (_, text_keys) = idea_history::effect_keys(&infos);
        let texts = self
            .content()
            .map(|content| idea_format::texts(content, &text_keys))
            .unwrap_or_default();
        let [info] = infos;
        LedgerIdeaTooltips {
            artwork: vec![dashboard_core::dto::Artwork {
                token: info.token.clone(),
                name: item.name.clone(),
                description: dated
                    .and_then(|dated| dated.facts.text.as_ref())
                    .and_then(|text| text.description.clone()),
                icon: None,
                base_duration_days: None,
            }],
            ideas: vec![info],
            modifiers,
            texts,
        }
    }

    /// The item and category a payment shows, with its tooltip: the item named by its first
    /// payment's id, and the warnings showing it gives counted in `presentation`.
    fn item_of(
        shown: &Shown,
        first: &str,
        presentation: &mut Presentation,
    ) -> (
        LedgerSpendItem,
        (String, String),
        Option<LedgerIdeaTooltips>,
    ) {
        let (item, _, category) = &shown.resolved;
        let mut item = item.clone();
        item.id = first.to_owned();
        presentation.formats.extend(shown.formats.iter().cloned());
        (item, category.clone(), shown.ideas.clone())
    }

    /// A power row read in the reply's hours: its balance line, and its payment when it is one.
    fn present_power<P: PageRead>(
        &mut self,
        reader: &mut Reader<'_, '_, P>,
        event: &PowerEvent,
        presentation: &mut Presentation,
    ) -> io::Result<Option<Presented>> {
        let row = &event.row;
        let classed = &event.classed;
        if classed.class == emit::SKIPPED {
            return Ok(None);
        }
        let ledger = rows::mutation(row, event.read, event.covered);
        let id = rows::mutation_id(row.country, row.epoch, row.sequence);
        let shown = self.shown(
            reader,
            classed.ordinal,
            &ledger,
            &row.identity,
            row.cause,
            event.dated,
            None,
        )?;
        let (mut item, category, ideas) = Self::item_of(&shown, &classed.first, presentation);
        if let Some(reason) = &item.unknown {
            rows::count_reason(&mut presentation.reasons, reason.clone(), 1)?;
        }
        presentation.balance.insert(
            (row.country, row.epoch, row.sequence),
            (item.action.clone(), item.unknown.clone()),
        );
        if classed.class != emit::PAYMENT {
            return Ok(None);
        }
        let charge = rows::charge(row).flatten();
        if row.currency == PowerCurrency::Command && row.cause == PowerCause::AbilityUpkeep {
            item.name = None;
            item.icon = None;
            item.icon_art = None;
            item.tooltip_token = None;
            item.character = None;
            item.unknown = None;
            item.action = "Ability upkeep".into();
        }
        let ideas = ideas.filter(|_| item.tooltip_token.is_some());
        let country_name = shown.country_name.clone();
        let change = LedgerSpendChange {
            hour: Some(row.hour),
            read_hour: event.read,
            charge_raw: charge.map(|raw| raw.to_string()),
            realized_delta_raw: Some(ledger.realized_delta_raw.clone()),
            occurrences: 1,
            covered: event.covered,
        };
        let spend = LedgerSpend {
            country_id: row.country,
            id,
            sequence: row.sequence.to_string(),
            ordinal: 0,
            hour: row.hour,
            read_hour: event.read,
            country_tag: self.tag.clone(),
            country_name,
            currency: ledger.currency.clone(),
            category: category.0.clone(),
            item_id: classed.first.clone(),
            charge_raw: charge.map(|raw| raw.to_string()),
            realized_delta_raw: Some(ledger.realized_delta_raw.clone()),
            covered: event.covered,
            occurrences: 1,
            changes: vec![change],
        };
        Ok(Some(Presented {
            contribution: Contribution {
                field: (classed.ordinal, row.country, row.epoch),
                recurring: classed.recurring,
                upkeep: classed.upkeep,
                hour: row.hour,
                read: event.read,
                order: (event.read, event.sequence),
                sequence: row.sequence,
                ordinal: 0,
                charge,
                covered: event.covered,
                realized: rows::amount(Some(ledger.realized_delta_raw.as_str()))?,
            },
            payment: Payment {
                known_raw: spend.charge_raw.clone().unwrap_or_else(|| "0".into()),
                known_count: u32::from(spend.charge_raw.is_some()),
                spend,
                item,
                category: LedgerSpendCategory {
                    identified: category.0 != "notIdentified",
                    id: category.0,
                    name: category.1,
                },
                ideas,
                count: 1,
                identity: (row.country, row.epoch.to_string()),
                ordinal: classed.ordinal,
            },
            recurring: classed.recurring,
            late: event.late,
        }))
    }

    /// A cost first recorded in the reply's hours, as the payment it is when it is one.
    fn present_cost<P: PageRead>(
        &mut self,
        reader: &mut Reader<'_, '_, P>,
        event: &CostEvent,
        presentation: &mut Presentation,
    ) -> io::Result<Option<Presented>> {
        let row = &event.row;
        let classed = &event.classed;
        if event.revision || classed.class == emit::SKIPPED {
            return Ok(None);
        }
        if classed.class == emit::UNMATCHED {
            rows::count_reason(
                &mut presentation.reasons,
                "Daily political power cost: matched PP application is unavailable".into(),
                1,
            )?;
            return Ok(None);
        }
        let id = rows::cost_id(row.country, row.epoch, row.day_hour, row.ordinal);
        let ledger = rows::cost(row, event.read, event.covered);
        let mutation = rows::daily_mutation(&ledger);
        let shown = self.shown(
            reader,
            classed.ordinal,
            &mutation,
            &row.identity,
            row.cause,
            event.dated,
            Some(row.component),
        )?;
        let (mut item, mut category, ideas) = Self::item_of(&shown, &classed.first, presentation);
        if classed.class == emit::CREDIT {
            let name = item.name.as_deref().unwrap_or("Not identified");
            rows::count_reason(
                &mut presentation.reasons,
                format!(
                    "Negative daily political power cost component {name}: recorded credit is not included in spending"
                ),
                1,
            )?;
            return Ok(None);
        }
        if row.cause == PowerCause::DailyGain {
            category = match row.component {
                PowerDailyCostKind::MioResearch | PowerDailyCostKind::MioProduction => {
                    ("organisationUpkeep".into(), "Organisation upkeep".into())
                }
                _ => ("dailyCosts".into(), "Daily costs".into()),
            };
            item.action = format!(
                "{}: {}",
                category.1,
                item.name.as_deref().unwrap_or("Not identified")
            );
        }
        if let Some(reason) = &item.unknown {
            rows::count_reason(&mut presentation.reasons, reason.clone(), 1)?;
        }
        let country_name = shown.country_name.clone();
        // The day's political application read by then dates the payment's read.
        let read = self
            .applications
            .get(&(row.country, row.day_hour))
            .into_iter()
            .flatten()
            .filter(|(read, _)| *read <= event.read)
            .filter_map(|(_, pp)| *pp)
            .max()
            .map_or(event.read, |read| read.max(event.read));
        let charge = Some(i128::from(row.cost_raw).to_string());
        let spend = LedgerSpend {
            country_id: row.country,
            id,
            sequence: row.pp_sequence.to_string(),
            ordinal: row.ordinal,
            hour: row.day_hour,
            read_hour: read,
            country_tag: self.tag.clone(),
            country_name,
            currency: "Political".into(),
            category: category.0.clone(),
            item_id: classed.first.clone(),
            charge_raw: charge.clone(),
            realized_delta_raw: None,
            covered: event.covered,
            occurrences: 1,
            changes: vec![LedgerSpendChange {
                hour: Some(row.day_hour),
                read_hour: read,
                charge_raw: charge,
                realized_delta_raw: None,
                occurrences: 1,
                covered: event.covered,
            }],
        };
        Ok(Some(Presented {
            contribution: Contribution {
                field: (classed.ordinal, row.country, row.epoch),
                recurring: classed.recurring,
                upkeep: classed.upkeep,
                hour: row.day_hour,
                read,
                order: (event.read, event.sequence),
                sequence: row.pp_sequence,
                ordinal: row.ordinal,
                charge: Some(i128::from(row.cost_raw)),
                covered: event.covered,
                realized: None,
            },
            payment: Payment {
                known_raw: i128::from(row.cost_raw).to_string(),
                known_count: 1,
                spend,
                item,
                category: LedgerSpendCategory {
                    identified: category.0 != "notIdentified",
                    id: category.0,
                    name: category.1,
                },
                ideas,
                count: 1,
                identity: (row.country, row.epoch.to_string()),
                ordinal: classed.ordinal,
            },
            recurring: classed.recurring,
            late: event.late,
        }))
    }

    /// The country's payments read, in the order they were recorded.
    fn present<P: PageRead>(
        &mut self,
        reader: &mut Reader<'_, '_, P>,
        cancelled: &AtomicBool,
    ) -> io::Result<Presentation> {
        let mut presentation = Presentation {
            payments: Vec::new(),
            balance: HashMap::new(),
            reasons: BTreeMap::new(),
            formats: BTreeSet::new(),
        };
        let power = std::mem::take(&mut self.power);
        for event in &power {
            cancelled_request(cancelled)?;
            if let Some(presented) = self.present_power(reader, event, &mut presentation)? {
                presentation.payments.push(presented);
            }
        }
        self.power = power;
        let costs = std::mem::take(&mut self.costs);
        for event in &costs {
            cancelled_request(cancelled)?;
            if let Some(presented) = self.present_cost(reader, event, &mut presentation)? {
                presentation.payments.push(presented);
            }
        }
        self.costs = costs;
        presentation
            .payments
            .sort_by_key(|presented| presented.contribution.order);
        if let Some(content) = self.content()
            && !presentation.payments.is_empty() | !presentation.balance.is_empty()
        {
            presentation.formats.extend(content.power_warnings().iter().cloned());
        }
        Ok(presentation)
    }
}

/// The facts of a row no frame of its hour was read for: its recorded key alone, and the text
/// content gives without a frame.
fn frameless(
    identity: &PowerIdentity,
    cause: PowerCause,
    component: Option<PowerDailyCostKind>,
    content: Option<&PoliticsContent>,
) -> Dated {
    let mut facts = items::event_facts(None, identity, cause, component);
    if let Some(content) = content {
        let texts = FrameTexts::default();
        facts.text = items::item_text(
            None,
            &Item { cause, identity },
            &facts,
            content,
            None,
            &texts,
        );
    }
    Dated {
        facts,
        own: None,
        target: None,
        modifiers: None,
    }
}

/// `payment` in `reply`: its item and category once, its tooltips merged, as a spend or a
/// recurring subtotal over `recurring` (first hour, last hour, days).
fn append_payment(
    reply: &mut PoliticsLedgerReply,
    mut payment: Payment,
    recurring: Option<(i32, i32, u32)>,
) -> io::Result<()> {
    if payment.item.tooltip_token.is_some() {
        payment.item.icon = None;
    }
    if !reply
        .spend_items
        .iter()
        .any(|item| item.id == payment.item.id)
    {
        reply.spend_items.push(payment.item);
    }
    if !reply
        .spend_categories
        .iter()
        .any(|category| category.id == payment.category.id)
    {
        reply.spend_categories.push(payment.category);
    }
    if let Some(ideas) = payment.ideas {
        let target = reply
            .idea_tooltips
            .get_or_insert_with(|| LedgerIdeaTooltips {
                ideas: Vec::new(),
                artwork: Vec::new(),
                modifiers: Vec::new(),
                texts: Vec::new(),
            });
        for info in ideas.ideas {
            if !target.ideas.iter().any(|old| old.token == info.token) {
                target.ideas.push(info);
            }
        }
        for art in ideas.artwork {
            if !target.artwork.iter().any(|old| old.token == art.token) {
                target.artwork.push(art);
            }
        }
        for modifier in ideas.modifiers {
            if !target.modifiers.iter().any(|old| old.key == modifier.key) {
                target.modifiers.push(modifier);
            }
        }
        for text in ideas.texts {
            if !target.texts.iter().any(|old| old.key == text.key) {
                target.texts.push(text);
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
            known_raw: payment.known_raw,
            known_count: payment.known_count,
        }),
        None => reply.spending.push(payment.spend),
    }
    Ok(())
}

/// One settled day of a recurring item: its merged payment, read at its latest contribution.
struct Day {
    payment: Payment,
    lag: i32,
    hour: i32,
    sequence: u64,
    ordinal: u32,
    realized_unknown: bool,
    realized: (i128, i128),
}

/// A recurring item's settled days by native day, and the template of its latest registration.
#[derive(Default)]
struct Recurring {
    days: BTreeMap<i32, Day>,
    /// Its contributions by read hour: the latest at a day's read names the subtotal.
    registered: BTreeMap<(i32, u64), Payment>,
    currency: String,
    category: String,
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
            known = known
                .checked_add(raw)
                .ok_or_else(|| invalid("Politics subtotal overflow"))?;
        }
    }
    payment.known_raw = known.to_string();
    payment.spend.covered = payment.spend.changes.iter().all(|change| change.covered);
    Ok(Some(payment))
}

/// What the country's payments add to a reply over `spans`, read by `to`: one-off spends, and
/// recurring subtotals by game month for settled days, by day for open or late ones.
#[allow(clippy::too_many_arguments)]
fn payments(
    reply: &mut PoliticsLedgerReply,
    presentation: &Presentation,
    settled: Option<i32>,
    view: &LedgerView,
    from: i32,
    to: i32,
    spans: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let boundary_days = boundary_days(spans);
    let settled_day = |day: i32| settled.is_some_and(|settled| day < settled);
    let mut items = BTreeMap::<String, Recurring>::new();
    let mut open = BTreeMap::<(i32, String), Payment>::new();
    let mut late = Vec::new();
    for presented in &presentation.payments {
        cancelled_request(cancelled)?;
        let payment = &presented.payment;
        if !presented.recurring {
            if payment.spend.read_hour >= from
                && payment.spend.read_hour <= to
                && selected(spans, payment.spend.hour)
            {
                append_payment(reply, payment.clone(), None)?;
            }
            continue;
        }
        let day = payment.spend.hour.div_euclid(24) * 24;
        let field = item_field(payment)?;
        if presented.late {
            if payment.spend.read_hour <= to && selected(spans, payment.spend.hour) {
                late.push(payment.clone());
            }
            continue;
        }
        if !settled_day(day) {
            match open.entry((day, field)) {
                std::collections::btree_map::Entry::Occupied(mut held) => {
                    merge_keeping(held.get_mut(), payment)?;
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(payment.clone());
                }
            }
            continue;
        }
        let recurring = items.entry(field).or_insert_with(|| Recurring {
            currency: payment.spend.currency.clone(),
            category: payment.spend.category.clone(),
            ..Recurring::default()
        });
        recurring.registered.insert(
            (payment.spend.read_hour, presented.contribution.order.1),
            payment.clone(),
        );
        let realized = rows::amount(payment.spend.realized_delta_raw.as_deref())?;
        match recurring.days.get_mut(&day) {
            Some(held) => {
                let realized_unknown = held.realized_unknown || realized.is_none();
                merge_keeping(&mut held.payment, payment)?;
                held.lag = held.lag.max(payment.spend.read_hour - day);
                held.realized_unknown = realized_unknown;
                if let Some(value) = realized {
                    held.realized.0 += value.max(0);
                    held.realized.1 += value.min(0);
                }
                if payment.spend.hour < held.hour {
                    held.hour = payment.spend.hour;
                }
            }
            None => {
                let merged = payment.clone();
                recurring.days.insert(
                    day,
                    Day {
                        lag: payment.spend.read_hour - day,
                        hour: payment.spend.hour,
                        sequence: payment.spend.sequence.parse().unwrap_or(0),
                        ordinal: payment.spend.ordinal,
                        realized_unknown: realized.is_none(),
                        realized: realized.map_or((0, 0), |value| (value.max(0), value.min(0))),
                        payment: merged,
                    },
                );
            }
        }
    }
    let mut category_days = BTreeMap::<(String, String), BTreeSet<i32>>::new();
    let mut cut = Vec::new();
    for (field, recurring) in &items {
        cancelled_request(cancelled)?;
        // A settled day is read once its latest contribution is; Command days a span cuts are
        // shown on their own.
        let mut visible: Vec<i32> = recurring
            .days
            .iter()
            .filter(|(day, held)| {
                let day = **day;
                selected(spans, day)
                    && i64::from(day) + i64::from(held.lag) <= i64::from(to)
                    && !(recurring.currency == "Command" && boundary_days.contains(&day))
            })
            .map(|(day, _)| *day)
            .collect();
        visible.sort_unstable();
        if recurring.currency == "Command" {
            for (day, held) in &recurring.days {
                if boundary_days.contains(day)
                    && let Some(clipped) = clip_command_payment(held.payment.clone(), spans, to)?
                {
                    cut.push(clipped);
                }
            }
        }
        category_days
            .entry((recurring.currency.clone(), recurring.category.clone()))
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
            let template = recurring
                .registered
                .range(..=(read_hour, u64::MAX))
                .next_back()
                .map(|(_, payment)| payment)
                .unwrap_or(&latest.payment);
            let mut payment = template.clone();
            let (mut count, mut entries, mut charge, mut unknown, mut uncovered) =
                (0_u32, 0_u32, 0_i128, 0_u32, 0_u32);
            let (mut realized_unknown, mut realized) = (false, (0_i128, 0_i128));
            for day in &days {
                let held = &recurring.days[day];
                count += held.payment.spend.occurrences;
                entries += held.payment.count;
                charge = charge
                    .checked_add(rows::amount(Some(&held.payment.known_raw))?.unwrap_or(0))
                    .ok_or_else(|| invalid("Politics subtotal overflow"))?;
                unknown += held.payment.spend.occurrences - held.payment.known_count;
                uncovered += u32::from(!held.payment.spend.covered);
                realized_unknown |= held.realized_unknown;
                realized.0 += held.realized.0;
                realized.1 += held.realized.1;
            }
            payment.spend.id = format!("recurring:{}:{field}:{month:?}:{first_hour}", reply.tag);
            payment.spend.hour = latest.hour;
            payment.spend.read_hour = read_hour;
            payment.spend.sequence = latest.sequence.to_string();
            payment.spend.ordinal = latest.ordinal;
            payment.spend.charge_raw = (unknown == 0).then(|| charge.to_string());
            payment.known_raw = charge.to_string();
            payment.known_count = count - unknown;
            payment.spend.realized_delta_raw = None;
            if (matches!(view, LedgerView::Raw | LedgerView::Balance) || month.is_none())
                && !realized_unknown
            {
                payment.spend.realized_delta_raw = Some((realized.0 + realized.1).to_string());
            }
            payment.spend.occurrences = count;
            payment.spend.covered = uncovered == 0;
            payment.spend.changes = vec![LedgerSpendChange {
                hour: Some(payment.spend.hour),
                read_hour: payment.spend.read_hour,
                charge_raw: payment.spend.charge_raw.clone(),
                realized_delta_raw: payment.spend.realized_delta_raw.clone(),
                occurrences: payment.spend.occurrences,
                covered: payment.spend.covered,
            }];
            payment.count = entries;
            append_payment(
                reply,
                payment,
                Some((first_hour, native_hour, days.len() as u32)),
            )?;
        }
    }
    late.extend(cut);
    // Open days: their payments as read by `to`, merged per day; Command days a span cuts keep
    // only what the span holds.
    for ((day, _), payment) in &open {
        let mut payment = payment.clone();
        if payment.spend.currency == "Command" && boundary_days.contains(day) {
            match clip_command_payment(payment, spans, to)? {
                Some(clipped) => payment = clipped,
                None => continue,
            }
        } else if !(payment.spend.read_hour <= to && selected(spans, payment.spend.hour)) {
            continue;
        }
        late.push(payment);
    }
    for mut payment in late {
        let day = payment.spend.hour.div_euclid(24) * 24;
        payment.spend.changes = vec![LedgerSpendChange {
            hour: Some(payment.spend.hour),
            read_hour: payment.spend.read_hour,
            charge_raw: payment.spend.charge_raw.clone(),
            realized_delta_raw: payment.spend.realized_delta_raw.clone(),
            occurrences: payment.spend.occurrences,
            covered: payment.spend.covered,
        }];
        category_days
            .entry((
                payment.spend.currency.clone(),
                payment.spend.category.clone(),
            ))
            .or_default()
            .insert(day);
        append_payment(reply, payment, Some((day, day, 1)))?;
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
    let _ = view;
    Ok(())
}

/// The first hour of the month window that holds `day` among `spans`.
fn month_window_start(spans: &[HourSpan], day: i32) -> i32 {
    for (_, windows) in month_spans(spans) {
        if windows
            .iter()
            .any(|window| window.first_hour <= day && day <= window.last_hour)
        {
            return windows[0].first_hour;
        }
    }
    day
}

/// The latest revision of each of `rows`' days within `spans` read by `to`.
fn latest_daily(rows: &[DailyEvent], spans: &[HourSpan], to: i32) -> Vec<LedgerDaily> {
    let mut latest = BTreeMap::<(u32, u64, i32), &DailyEvent>::new();
    for row in rows {
        if row.read > to || !selected(spans, row.row.day_hour) {
            continue;
        }
        latest.insert((row.row.country, row.row.epoch, row.row.day_hour), row);
    }
    let mut daily: Vec<LedgerDaily> = latest
        .values()
        .map(|row| rows::daily(&row.row, row.read, row.reads))
        .collect();
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

/// A balance line's action for a mutation the item does not name.
fn fixed_action(row: &CountryPowerChangeRow) -> Option<&'static str> {
    Some(match row.cause {
        PowerCause::DailyGain => {
            if row.currency == PowerCurrency::Command
                && row.requested_delta_raw.is_some_and(|raw| raw < 0)
            {
                "Daily deficit"
            } else {
                "Daily income applied"
            }
        }
        PowerCause::CapCorrection => "Maximum correction",
        PowerCause::ResetSetup => "Balance set/reset",
        PowerCause::Console => "Console adjustment",
        _ => return None,
    })
}

/// The capture coverage of the country over `from..=to`: each run of its capture fact.
fn coverage<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    from: i32,
    to: i32,
) -> io::Result<Vec<LedgerCoverage>> {
    let directory = reader.directory;
    let facts_key = keys.key(directory, CAPTURE_FACTS);
    let mut facts = HashMap::new();
    for event in reader.events(facts_key, i32::MIN, to)? {
        let cells = reader.cells(event.fields)?;
        let reason = reader.text(cells.0.get(6))?;
        facts.insert(
            event.sequence,
            LedgerCoverage {
                country: cells.u32(0),
                instance: cells.u32(1),
                read_hour: Cells::required(cells.i32(2))?,
                since_hour: cells.i32(3),
                through_hour: None,
                epoch: cells.unsigned(4).map(|epoch| epoch.to_string()),
                streams: cells
                    .unsigned(5)
                    .and_then(|streams| u8::try_from(streams).ok())
                    .unwrap_or(0),
                reason,
            },
        );
    }
    let mut rows = Vec::new();
    for run in reader.runs(keys.key(directory, CAPTURE), from, to)? {
        let Some(fact) = u64::try_from(run.value).ok().and_then(|id| facts.get(&id)) else {
            continue;
        };
        let mut row = fact.clone();
        row.since_hour = Some(row.since_hour.unwrap_or(run.span.first).max(from));
        row.through_hour = Some(run.span.last);
        rows.push(row);
    }
    Ok(rows)
}

/// The country's spending coverage over `from..=to`: where its census, presence and coverage runs
/// say its spending is recorded, why not elsewhere.
fn spending_coverage<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    keys: &Keys,
    tag: &str,
    from: i32,
    to: i32,
) -> io::Result<Vec<LedgerSpendCoverage>> {
    let directory = reader.directory;
    let branch = directory.actor(emit::BRANCH);
    let branch_key = |measure| branch.and_then(|actor| directory.key(actor, measure, None));
    let census = reader.runs(branch_key(emit::CENSUS), from, to)?;
    let mut presence = BTreeMap::<i32, (bool, bool)>::new();
    let presence_key = branch_key(PRESENCE);
    let mut presence_events = Vec::new();
    if let Some(event) = reader.latest(presence_key, from)? {
        presence_events.push(event);
    }
    if from < to {
        presence_events.extend(reader.events(presence_key, from + 1, to)?);
    }
    for event in presence_events {
        let cells = reader.cells(event.fields)?;
        let mut lists = [false; 2];
        for (index, list) in lists.iter_mut().enumerate() {
            if let Some(Cell::List(blob)) = cells.0.get(index) {
                let tags = reader.cells(*blob)?;
                for cell in &tags.0 {
                    if reader.bytes(Some(cell))?.as_deref() == Some(tag.as_bytes()) {
                        *list = true;
                    }
                }
            }
        }
        presence.insert(event.native_hour, (lists[0], lists[1]));
    }
    let mut boundaries = BTreeSet::from([i64::from(from), i64::from(to) + 1]);
    let mut presence_boundaries: BTreeSet<i64> = census
        .iter()
        .flat_map(|run| [i64::from(run.span.first), i64::from(run.span.last) + 1])
        .collect();
    let mut previous = None;
    for (&hour, value) in &presence {
        if previous != Some(*value) && hour >= from {
            presence_boundaries.insert(i64::from(hour));
        }
        previous = Some(*value);
    }
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
    let mut reason_events = Vec::new();
    if let Some(event) = reader.latest(reasons_key, from)? {
        reason_events.push(event);
    }
    if from < to {
        for event in reader.events(reasons_key, from.saturating_add(1), to)? {
            boundaries.insert(i64::from(event.native_hour));
            reason_events.push(event);
        }
    }
    let mut reasons = BTreeMap::<i32, LedgerSpendCoverage>::new();
    for event in reason_events {
        let cells = reader.cells(event.fields)?;
        let reason = reader.text(cells.0.get(5))?;
        reasons.insert(
            event.native_hour,
            LedgerSpendCoverage {
                read_hour: Cells::required(cells.i32(0))?,
                since_hour: cells.i32(1),
                through_hour: cells.i32(2),
                political: cells.boolean(3),
                command: cells.boolean(4),
                reason,
            },
        );
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
    tag: &str,
    content: Option<&PoliticsContent>,
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
    let presence_key = branch_key(PRESENCE);
    let mut snapshots = Vec::new();
    for event in events {
        let cells = reader.cells(event.fields)?;
        let total = cells.i64(0);
        let missing = reader.text(cells.0.get(2))?;
        let mut holders = Vec::new();
        let mut reasons = BTreeMap::<String, usize>::new();
        let mut attributed = 0_i128;
        if let Some(Cell::List(blob)) = cells.0.get(1) {
            let list = reader.cells(*blob)?;
            for cell in &list.0 {
                let Cell::List(blob) = cell else {
                    continue;
                };
                let holder = reader.cells(*blob)?;
                let id = reader.text(holder.0.first())?.unwrap_or_default();
                let kind = reader.text(holder.0.get(1))?.unwrap_or_default();
                let ability = holder.boolean(2);
                let token = reader.text(holder.0.get(3))?;
                let script = reader.text(holder.0.get(4))?;
                let saved = reader.text(holder.0.get(5))?;
                let amount = holder.integer(6).unwrap_or(0);
                attributed += amount;
                let name = match present::holder_name(
                    ability,
                    token.as_deref(),
                    saved.as_deref().map(|saved| (script.as_deref(), saved)),
                    &kind,
                    content,
                ) {
                    Ok(name) => Some(name),
                    Err(reason) => {
                        *reasons.entry(reason).or_default() += 1;
                        None
                    }
                };
                holders.push(LedgerCommandHolder {
                    id,
                    kind,
                    name,
                    amount_raw: Some(amount.to_string()),
                });
            }
        }
        let row = match total {
            None => LedgerCommandHold {
                read_hour: event.native_hour,
                through_hour: event.native_hour,
                total_raw: None,
                holders: Vec::new(),
                reason: Some(missing.unwrap_or_else(|| "CountryCommandPower unavailable".into())),
            },
            Some(total) => {
                let remainder = i128::from(total) - attributed;
                if remainder != 0 {
                    holders.push(LedgerCommandHolder {
                        id: "notIdentified".into(),
                        kind: "Not identified".into(),
                        name: None,
                        amount_raw: Some(remainder.to_string()),
                    });
                    *reasons
                        .entry(
                            "Command power holder identity is not recorded for part of the held amount"
                                .into(),
                        )
                        .or_default() += 1;
                }
                LedgerCommandHold {
                    read_hour: event.native_hour,
                    through_hour: event.native_hour,
                    total_raw: Some(total.to_string()),
                    holders,
                    reason: (!reasons.is_empty()).then(|| {
                        reasons
                            .iter()
                            .map(|(reason, count)| format!("{reason} (count: {count})"))
                            .collect::<Vec<_>>()
                            .join("; ")
                    }),
                }
            }
        };
        snapshots.push((event.native_hour, row));
    }
    // A snapshot of a present country runs to the reply's last hour; the next closes it.
    let mut presence = BTreeMap::<i32, bool>::new();
    let mut presence_events = Vec::new();
    if let Some(event) = reader.latest(presence_key, from)? {
        presence_events.push(event);
    }
    if from < to {
        presence_events.extend(reader.events(presence_key, from + 1, to)?);
    }
    for event in presence_events {
        let cells = reader.cells(event.fields)?;
        let mut present = false;
        if let Some(Cell::List(blob)) = cells.0.first() {
            let tags = reader.cells(*blob)?;
            for cell in &tags.0 {
                if reader.bytes(Some(cell))?.as_deref() == Some(tag.as_bytes()) {
                    present = true;
                }
            }
        }
        presence.insert(event.native_hour, present);
    }
    let mut rows: Vec<LedgerCommandHold> = Vec::new();
    for (hour, mut row) in snapshots {
        let at = hour.max(from);
        let present = census
            .iter()
            .any(|run| run.span.first <= at && at <= run.span.last)
            && presence
                .range(..=at)
                .next_back()
                .is_some_and(|(_, present)| *present);
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
        rows.push(decode_government(reader, &event)?);
    }
    rows.sort_by_key(|row| (row.read_hour, row.country, row.sequence.parse::<u64>().ok()));
    Ok(rows)
}

/// The reply to a ledger request, from the branch's publication.
pub fn query<P: PageRead>(
    params: &Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    content: Option<&Arc<PoliticsContent>>,
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
    };
    let keys = Keys::of(directory, &params.tag);
    if let Some(navigation) = &params.navigation
        && params.view != LedgerView::Governments
    {
        return navigate(&mut reader, &params, navigation);
    }
    if let Some(view) = &params.summary {
        return summaries(&mut reader, &params, view);
    }
    let to = params.hour.unwrap_or(reader.last);
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
                let Some(read) = event.read_hour.filter(|read| *read >= from && *read <= end)
                else {
                    continue;
                };
                let _ = read;
                let row = decode_government(&mut reader, &event)?;
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
    let mut country = Country::read(&mut reader, &params.tag, content, &plan.raw)?;
    let mut presentation = country.present(&mut reader, cancelled)?;
    let mut reply = PoliticsLedgerReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: 5,
        ..Default::default()
    };
    let view = params.view.clone();
    if matches!(view, LedgerView::Raw | LedgerView::Balance) {
        for event in &country.power {
            if event.read < from || event.read > to || !selected(&spans, event.row.hour) {
                continue;
            }
            let row = &event.row;
            let ledger = rows::mutation(row, event.read, event.covered);
            let (action, reason) = match fixed_action(row) {
                Some(action) => (action.to_owned(), None),
                None => presentation
                    .balance
                    .get(&(row.country, row.epoch, row.sequence))
                    .cloned()
                    .unwrap_or_else(|| ("Not identified".into(), None)),
            };
            reply.balance_changes.push(LedgerBalanceChange {
                country_id: row.country,
                hour: row.hour,
                read_hour: event.read,
                currency: ledger.currency.clone(),
                amount_raw: ledger.amount_raw.clone(),
                realized_delta_raw: ledger.realized_delta_raw.clone(),
                action,
                covered: event.covered,
                reason,
            });
            if view == LedgerView::Raw {
                reply.mutations.push(ledger);
            }
        }
    }
    if view == LedgerView::Raw {
        reply.governments = governments(&mut reader, &keys, from, to, &spans)?;
        let mut costs = BTreeMap::<(u32, u64, i32, u32), &CostEvent>::new();
        for event in &country.costs {
            if event.read > to || !selected(&spans, event.row.day_hour) {
                continue;
            }
            costs.insert(
                (
                    event.row.country,
                    event.row.epoch,
                    event.row.day_hour,
                    event.row.ordinal,
                ),
                event,
            );
        }
        // A revision keeps its first record's covered flag.
        let mut first_covered = HashMap::new();
        for event in country.costs.iter().filter(|event| !event.revision) {
            first_covered.insert(
                (
                    event.row.country,
                    event.row.epoch,
                    event.row.day_hour,
                    event.row.ordinal,
                ),
                event.covered,
            );
        }
        reply.daily_costs = costs
            .iter()
            .map(|(key, event)| {
                rows::cost(
                    &event.row,
                    event.read,
                    first_covered.get(key).copied().unwrap_or(event.covered),
                )
            })
            .collect();
        reply.coverage = coverage(&mut reader, &keys, from, to)?;
    }
    let daily = latest_daily(&country.daily, &spans, to);
    reply.spending_coverage = spending_coverage(&mut reader, &keys, &params.tag, from, to)?;
    if matches!(view, LedgerView::Raw | LedgerView::Balance) {
        let named = content.map(|content| &**content);
        reply.command_holds = holds(&mut reader, &keys, &params.tag, named, from, to)?;
    }
    let held = if rolled {
        totals(
            &mut reply,
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
        payments(
            &mut reply,
            &presentation,
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
    if matches!(view, LedgerView::Raw | LedgerView::Balance) {
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
    reply.daily_costs.sort_by(|a, b| {
        (a.day_hour, a.country, &a.epoch, a.ordinal)
            .cmp(&(b.day_hour, b.country, &b.epoch, b.ordinal))
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
    reader: &mut Reader<'_, '_, P>,
    country: &mut Country<'_>,
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
    let mut extra = Vec::new();
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
            let presented = match event.kind {
                emit::POWER_KIND => {
                    let event = decode_power(reader, &event)?;
                    country.present_power(reader, &event, presentation)?
                }
                emit::COST_KIND => {
                    let event = decode_cost(reader, &event)?;
                    country.present_cost(reader, &event, presentation)?
                }
                _ => {
                    stale = true;
                    None
                }
            };
            extra.extend(presented);
        }
        if stale {
            let mut daily = Vec::new();
            for event in reader.events(daily_key, *first, *last)? {
                daily.push(decode_daily(reader, &event)?);
            }
            daily.sort_by_key(|event| (event.row.day_hour, event.read));
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
    let settled = reader.settled;
    let mut templates: HashMap<Field, Payment> = HashMap::new();
    let mut days: BTreeMap<(i32, Field), (tally::Day, Payment)> = BTreeMap::new();
    let mut open = BTreeMap::<(i32, String), Payment>::new();
    let mut individual = Vec::new();
    for presented in presentation.payments.iter().chain(&extra) {
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
            match open.entry((day, item_field(payment)?)) {
                std::collections::btree_map::Entry::Occupied(mut held) => {
                    merge_keeping(held.get_mut(), payment)?;
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
                merge_keeping(merged, payment)?;
            }
            None => {
                days.insert(
                    (day, contribution.field),
                    (tally::Day::new(contribution), payment.clone()),
                );
            }
        }
    }
    for ((day, field), (held, merged)) in &days {
        let command = merged.spend.currency == "Command";
        if command && boundary.contains(day) {
            if let Some(clipped) = clip_command_payment(merged.clone(), spans, to)? {
                individual.push(clipped);
            }
            continue;
        }
        if selected(spans, *day) && i64::from(*day) + i64::from(held.lag) <= i64::from(to) {
            entries.entry(*field).or_default().add_day(held)?;
        }
    }
    for ((day, _), payment) in &open {
        let mut payment = payment.clone();
        if payment.spend.currency == "Command" && boundary.contains(day) {
            match clip_command_payment(payment, spans, to)? {
                Some(clipped) => payment = clipped,
                None => continue,
            }
        } else if !(payment.spend.read_hour <= to && selected(spans, payment.spend.hour)) {
            continue;
        }
        individual.push(payment);
    }
    // Items only rollups hold are named by their first payment.
    let missing: BTreeSet<u32> = entries
        .keys()
        .filter(|field| !templates.contains_key(*field))
        .map(|field| field.0)
        .collect();
    let mut catalogue = HashMap::<u32, Payment>::new();
    if !missing.is_empty() {
        let key = country.keys.key(reader.directory, ITEMS);
        let mut scratch = Presentation {
            payments: Vec::new(),
            balance: HashMap::new(),
            reasons: BTreeMap::new(),
            formats: BTreeSet::new(),
        };
        for event in reader.events(key, reader.first, to)? {
            cancelled_request(cancelled)?;
            let presented = match event.kind {
                emit::POWER_KIND => {
                    let mut event = decode_power(reader, &event)?;
                    if !missing.contains(&event.classed.ordinal) {
                        continue;
                    }
                    event.classed.class = emit::PAYMENT;
                    country.present_power(reader, &event, &mut scratch)?
                }
                emit::COST_KIND => {
                    let mut event = decode_cost(reader, &event)?;
                    if !missing.contains(&event.classed.ordinal) {
                        continue;
                    }
                    event.classed.class = emit::PAYMENT;
                    event.revision = false;
                    country.present_cost(reader, &event, &mut scratch)?
                }
                _ => None,
            };
            if let Some(presented) = presented {
                catalogue
                    .entry(presented.contribution.field.0)
                    .or_insert(presented.payment);
            }
        }
        presentation.formats.extend(scratch.formats);
    }
    let mut fields: Vec<Field> = entries.keys().copied().collect();
    fields.sort_unstable();
    for field in fields {
        let entry = entries[&field];
        let template = match templates.get(&field) {
            Some(template) => template.clone(),
            None => {
                let mut template = catalogue.get(&field.0).cloned().ok_or_else(|| {
                    invalid("A politics item's first payment is missing from its catalogue")
                })?;
                template.identity = (field.1, field.2.to_string());
                template.spend.country_id = field.1;
                template
            }
        };
        // What rollups count of a line a reply shows says why its item is not named.
        if let Some(rolled) = rolled.get(&field) {
            let paid = rolled.one_off.map_or(0, |one| one.occurrences)
                + rolled
                    .recurring
                    .map_or(0, |recurring| recurring.occurrences);
            let unknown = country
                .shown
                .get(&field.0)
                .and_then(|shown| shown.resolved.0.unknown.clone());
            if let Some(reason) = &unknown
                && paid + rolled.balances > 0
            {
                rows::count_reason(
                    &mut presentation.reasons,
                    reason.clone(),
                    paid + rolled.balances,
                )?;
            }
            if rolled.credits > 0 {
                let name = template.item.name.as_deref().unwrap_or("Not identified");
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
        let count = |value: u64| {
            u32::try_from(value).map_err(|_| invalid("Politics payment count overflow"))
        };
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
            payment.known_raw = one.known.to_string();
            payment.known_count = count(one.occurrences - one.unknown)?;
            payment.count = count(one.entries)?;
            payment.spend.changes = vec![LedgerSpendChange {
                hour: Some(payment.spend.hour),
                read_hour: payment.spend.read_hour,
                charge_raw: payment.spend.charge_raw.clone(),
                realized_delta_raw: None,
                occurrences: payment.spend.occurrences,
                covered: payment.spend.covered,
            }];
            append_payment(reply, payment, Some((from, one.latest.0, 0)))?;
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
            payment.known_raw = recurring.charge.to_string();
            payment.known_count = count(recurring.occurrences - recurring.unknown)?;
            payment.spend.realized_delta_raw = (!recurring.realized_unknown)
                .then(|| (recurring.realized.0 + recurring.realized.1).to_string());
            payment.spend.occurrences = count(recurring.occurrences)?;
            payment.spend.covered = recurring.uncovered == 0;
            payment.spend.changes = vec![LedgerSpendChange {
                hour: Some(payment.spend.hour),
                read_hour: payment.spend.read_hour,
                charge_raw: payment.spend.charge_raw.clone(),
                realized_delta_raw: payment.spend.realized_delta_raw.clone(),
                occurrences: payment.spend.occurrences,
                covered: payment.spend.covered,
            }];
            payment.count = count(recurring.entries)?;
            append_payment(
                reply,
                payment,
                Some((first_hour, native_hour, count(recurring.days)?)),
            )?;
        }
    }
    for mut payment in individual {
        let day = payment.spend.hour.div_euclid(24) * 24;
        payment.spend.changes = vec![LedgerSpendChange {
            hour: Some(payment.spend.hour),
            read_hour: payment.spend.read_hour,
            charge_raw: payment.spend.charge_raw.clone(),
            realized_delta_raw: payment.spend.realized_delta_raw.clone(),
            occurrences: payment.spend.occurrences,
            covered: payment.spend.covered,
        }];
        append_payment(reply, payment, Some((day, day, 1)))?;
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

/// The keys a navigation request walks: the summary's parts, or the country's income, coverage,
/// payments, governments and holds.
fn navigate<P: PageRead>(
    reader: &mut Reader<'_, '_, P>,
    params: &Params,
    navigation: &Navigation,
) -> io::Result<Value> {
    let directory = reader.directory;
    let tags: Vec<&String> = params
        .bindings
        .as_ref()
        .map(|bindings| bindings.keys().collect())
        .unwrap_or_else(|| vec![&params.tag]);
    let mut selected = Vec::new();
    for tag in tags {
        let keys = Keys::of(directory, tag);
        let within = match &params.bindings {
            Some(bindings) => bindings.get(tag).map(Vec::as_slice),
            None => params.within.as_deref(),
        };
        let mut found = Vec::new();
        if let Some(view) = &params.summary {
            let currency = usize::from(view.currency == "Command");
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
        } else {
            found.extend(
                INCOME
                    .iter()
                    .filter_map(|&measure| keys.key(directory, measure)),
            );
            found.extend(
                COVERED
                    .iter()
                    .filter_map(|&measure| keys.key(directory, measure)),
            );
            found.extend(
                POWER
                    .iter()
                    .filter_map(|&measure| keys.key(directory, measure)),
            );
            found.extend(keys.key(directory, GOVERNMENTS));
            found.extend(keys.key(directory, HOLDS));
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
        let row = decode_government(reader, event)?;
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
