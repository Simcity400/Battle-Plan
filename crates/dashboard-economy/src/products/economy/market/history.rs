use super::{LeaseAmount, LeaseHour, LeaseItems, LeaseTerms, pool, pool_variants};
use dashboard_base::{Source, products::{BranchParams, LoadContext, Product, check_reply_budget}, queries::country_ids::Names};
use dashboard_world::products::equipment_names::{EquipmentIcon, VariantName, self as names, Named};
use replay_lanes::lanes::{self, Keys};
use replay_columnar::store::TableView;
use replay_data::{
    codec::Decoder,
    rows::{MarketPool, SharedMarketPool, TableId},
    value::Value as _,
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolMissing {
    Unrecorded,
    Corrupt,
}

impl PoolMissing {
    pub(crate) fn reason(self, when: &str) -> String {
        match self {
            Self::Unrecorded => format!("Not recorded {when}"),
            Self::Corrupt => format!("Could not be decoded {when}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndedRow {
    pub country: u32,
    pub id: u64,
    pub receiver: Option<u32>,
    pub revision: Option<u64>,
    pub active_since: Option<i32>,
    pub last_delivery: Option<i32>,
    pub monthly: Result<SharedMarketPool, PoolMissing>,
    pub percentage: Result<SharedMarketPool, PoolMissing>,
    pub once: Result<SharedMarketPool, PoolMissing>,
    pub delivered: Result<SharedMarketPool, PoolMissing>,
    pub sunk: Result<SharedMarketPool, PoolMissing>,
    pub fuel_daily_raw: Option<i64>,
    pub fuel_percentage_raw: Option<i64>,
    pub fuel_sent_raw: Option<i64>,
    pub fuel_sunk_raw: Option<i64>,
    pub last_fuel_delivered_raw: Option<i64>,
}

impl EndedRow {
    pub(crate) fn key(&self) -> u128 {
        u128::from(self.country) << 64 | u128::from(self.id)
    }

    pub(crate) fn variants(&self) -> impl Iterator<Item = u64> + '_ {
        [&self.monthly, &self.percentage, &self.once, &self.delivered, &self.sunk]
            .into_iter()
            .filter_map(|held| held.as_ref().ok())
            .flat_map(|held| pool_variants(held))
    }

    pub(crate) fn terms(&self, tag: impl FnMut(u32) -> Option<String>, named: &Named) -> LeaseTerms {
        self.terms_at("at the agreement's last read", tag, named)
    }

    pub(crate) fn terms_at(&self, when: &str, mut tag: impl FnMut(u32) -> Option<String>, named: &Named) -> LeaseTerms {
        let amount_u64 = |value: Option<u64>, label: &str| match value {
            Some(value) => LeaseAmount::known(value),
            None => LeaseAmount::missing(format!("{label} not recorded {when}")),
        };
        let amount_i64 = |value: Option<i64>, label: &str| match value {
            Some(value) => LeaseAmount::known(value),
            None => LeaseAmount::missing(format!("{label} not recorded {when}")),
        };
        let hour = |value: Option<i32>, label: &str| match value {
            Some(value) => LeaseHour::known(value),
            None => LeaseHour::missing(format!("{label} not recorded {when}")),
        };
        let items = |held: &Result<SharedMarketPool, PoolMissing>| match held {
            Ok(held) => LeaseItems::known(pool(named, held)),
            Err(why) => LeaseItems::missing(why.reason(when)),
        };
        let (receiver, receiver_missing) = match self.receiver {
            Some(raw) => (tag(raw), None),
            None => (None, Some(format!("Receiver not recorded {when}"))),
        };
        LeaseTerms {
            key: self.key().to_string(),
            id: format!("{:016x}", self.id),
            giver: tag(self.country),
            receiver,
            receiver_missing,
            revision: amount_u64(self.revision, "Revision"),
            active_since: hour(self.active_since, "Start date"),
            last_delivery: hour(self.last_delivery, "Last delivery date"),
            monthly: items(&self.monthly),
            percentage: items(&self.percentage),
            once: items(&self.once),
            delivered: items(&self.delivered),
            sunk: items(&self.sunk),
            fuel_daily_raw: amount_i64(self.fuel_daily_raw, "Daily fuel"),
            fuel_percentage_raw: amount_i64(self.fuel_percentage_raw, "Fuel production share"),
            last_fuel_delivered_raw: amount_i64(self.last_fuel_delivered_raw, "Last fuel delivery"),
        }
    }

    pub(crate) fn fuel_sent(&self) -> LeaseAmount {
        match self.fuel_sent_raw {
            Some(value) => LeaseAmount::known(value),
            None => LeaseAmount::missing("Fuel sent not recorded at the agreement's last read"),
        }
    }

    pub(crate) fn fuel_sunk(&self) -> LeaseAmount {
        match self.fuel_sunk_raw {
            Some(value) => LeaseAmount::known(value),
            None => LeaseAmount::missing("Fuel lost not recorded at the agreement's last read"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub ended_by: i32,
    pub row: EndedRow,
    pub whole: bool,
}

#[derive(Default)]
struct Runs {
    runs: HashMap<u32, Arc<Vec<Ended>>>,
}

pub(crate) struct Columns {
    pub(crate) numbers: [usize; 9],
    pub(crate) pools: [usize; 5],
}

impl Columns {
    pub(crate) fn read() -> io::Result<Self> {
        let field = |name| dashboard_declare::field(TableId::LendLeaseAgreements, name);
        Ok(Self {
            numbers: [
                field("receiver")?,
                field("revision")?,
                field("active_since")?,
                field("last_delivery")?,
                field("fuel_daily_raw")?,
                field("fuel_percentage_raw")?,
                field("fuel_sent_raw")?,
                field("fuel_sunk_raw")?,
                field("last_fuel_delivered_raw")?,
            ],
            pools: [field("monthly")?, field("percentage")?, field("once")?, field("delivered")?, field("sunk")?],
        })
    }

    pub(crate) fn row(&self, view: &TableView<'_>, slot: u32, key: u128) -> (EndedRow, bool) {
        let number = |at: usize| -> Option<i64> { view.number(self.numbers[at], slot) };
        let pool = |at: usize| -> Result<SharedMarketPool, PoolMissing> {
            match view.list(self.pools[at], slot) {
                None => Err(PoolMissing::Unrecorded),
                Some(bytes) => {
                    let mut input = Decoder::new(bytes);
                    match MarketPool::decode(&mut input).and_then(|pool| input.finish().map(|()| pool)) {
                        Ok(pool) => Ok(pool.into()),
                        Err(_) => Err(PoolMissing::Corrupt),
                    }
                }
            }
        };
        let row = EndedRow {
            country: (key >> 64) as u32,
            id: key as u64,
            receiver: number(0).and_then(|value| u32::try_from(value).ok()),
            revision: number(1).map(|value| value as u64),
            active_since: number(2).and_then(|value| i32::try_from(value).ok()),
            last_delivery: number(3).and_then(|value| i32::try_from(value).ok()),
            monthly: pool(0),
            percentage: pool(1),
            once: pool(2),
            delivered: pool(3),
            sunk: pool(4),
            fuel_daily_raw: number(4),
            fuel_percentage_raw: number(5),
            fuel_sent_raw: number(6),
            fuel_sunk_raw: number(7),
            last_fuel_delivered_raw: number(8),
        };
        let whole = row.receiver.is_some()
            && row.revision.is_some()
            && row.active_since.is_some()
            && row.last_delivery.is_some()
            && row.monthly.is_ok()
            && row.percentage.is_ok()
            && row.once.is_ok()
            && row.delivered.is_ok()
            && row.sunk.is_ok()
            && row.fuel_daily_raw.is_some()
            && row.fuel_percentage_raw.is_some()
            && row.fuel_sent_raw.is_some()
            && row.fuel_sunk_raw.is_some()
            && row.last_fuel_delivered_raw.is_some();
        (row, whole)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndedSpan {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub ended_by: i32,
}

pub fn ended_spans(history: &[(u128, Vec<lanes::Step>)], last_hour: i32, reads: &[(i32, i32)], truncated: &[(i32, i32)]) -> Vec<EndedSpan> {
    let first_read_in = |from: i32, to: i32| -> Option<i32> {
        if from > to {
            return None;
        }
        let idx = reads.partition_point(|&(_, b)| b < from);
        let &(a, b) = reads.get(idx)?;
        (a <= to && b >= from).then(|| a.max(from))
    };
    let in_runs = |runs: &[(i32, i32)], hour: i32| -> bool {
        let idx = runs.partition_point(|&(_, b)| b < hour);
        runs.get(idx).is_some_and(|&(a, b)| a <= hour && hour <= b)
    };
    let mut out = Vec::new();
    for (key, steps) in history {
        let mut open: Option<i32> = None;
        for (at, step) in steps.iter().enumerate() {
            match (&step.cells, open) {
                (Some(_), None) => open = Some(step.hour),
                (None, Some(first)) => {
                    let next_present = steps[at + 1..].iter().find(|later| later.cells.is_some());
                    let absent_to = next_present.map_or(last_hour, |later| later.hour - 1);
                    let ended_by = if next_present.is_none() && in_runs(truncated, step.hour) {
                        Some(step.hour)
                    } else {
                        first_read_in(step.hour, absent_to)
                    };
                    if let Some(ended_by) = ended_by {
                        out.push(EndedSpan { key: *key, first, last: step.hour - 1, ended_by });
                        open = None;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

pub fn run_ended(source: &Source, run: u32) -> io::Result<Arc<Vec<Ended>>> {
    let state = source.state::<Runs>();
    if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).runs.get(&run) {
        return Ok(Arc::clone(found));
    }
    let recording = source.replay_ref().recording();
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.last_hour());
    let columns = Columns::read()?;
    let table = TableId::LendLeaseAgreements;
    let history = dashboard_query::cost::storage(|| lanes::history(recording, run, table, &[columns.numbers[1]], Keys::All, first, last))?;
    let reads = dashboard_query::cost::storage(|| lanes::reads(recording, run, table, first, last))?;
    let truncated: Vec<(i32, i32)> = dashboard_query::cost::storage(|| lanes::statuses(recording, run, table, first, last))?
        .into_iter()
        .filter(|run| matches!(run.status, replay_columnar::manifest::Status::Truncated { .. }))
        .map(|run| (run.first, run.last))
        .collect();
    let spans = ended_spans(&history.keys, last, &reads, &truncated);
    let lives: HashMap<(i32, u128), (i32, i32)> = spans.iter().map(|span| ((span.last, span.key), (span.first, span.ended_by))).collect();
    let asks: Vec<(i32, u128)> = lives.keys().copied().collect();
    let mut ended = Vec::with_capacity(asks.len());
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, table, None, &asks, &mut |hour, key, row| {
            if let (Some((view, slot)), Some(&(first, ended_by))) = (row, lives.get(&(hour, key))) {
                let (row, whole) = columns.row(&view, slot, key);
                ended.push(Ended { key, first, last: hour, ended_by, row, whole });
            }
            Ok(())
        })
    })?;
    ended.sort_unstable_by_key(|ended| (ended.last, ended.key));
    let made = Arc::new(ended);
    state.lock().unwrap_or_else(PoisonError::into_inner).runs.insert(run, Arc::clone(&made));
    Ok(made)
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EndedLease {
    pub terms: LeaseTerms,
    pub fuel_sent_raw: LeaseAmount,
    pub fuel_sunk_raw: LeaseAmount,
    #[doc = " The first hour it stood in its run, within the branch."]
    pub first_hour: i32,
    #[doc = " Its last witnessed hour."]
    pub last_hour: i32,
    #[doc = " The first census hour that established the agreement absent."]
    pub ended_by: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseHistoryReply {
    pub branch_id: String,
    #[doc = " By the hour each ended, then key."]
    pub agreements: Vec<EndedLease>,
    #[doc = " Equipment named at the end of each stretch of the branch."]
    pub variants: Vec<VariantName>,
    pub icons: Vec<EquipmentIcon>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LendLeaseHistoryReply);

pub struct LendLeaseHistory;

impl Product for LendLeaseHistory {
    const KEY: &'static str = "lend-lease-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = LendLeaseHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EndedLease::decl(), LendLeaseHistoryReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let recording = query.replay.recording();
        let tags = Names::new(&query.holders);
        let context = names::context(source, load)?;
        let mut named = Named::default();
        let mut agreements = Vec::new();
        let mut partial = 0usize;
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let ended = run_ended(source, stretch.run)?;
            let mine: Vec<&Ended> = ended.iter().filter(|ended| stretch.first <= ended.last && ended.ended_by <= stretch.last).collect();
            if mine.is_empty() {
                continue;
            }
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let variants: BTreeSet<u64> = mine.iter().flat_map(|ended| ended.row.variants()).collect();
            let variants: Vec<u64> = variants.into_iter().collect();
            names::name_at(recording, stretch.run, stretch.last, &variants, &[], &context, &mut named)?;
            for ended in mine {
                partial += usize::from(!ended.whole);
                agreements.push(EndedLease {
                    terms: ended.row.terms(|raw| tags.tag_at(&links, raw, ended.last).map(str::to_owned), &named),
                    fuel_sent_raw: ended.row.fuel_sent(),
                    fuel_sunk_raw: ended.row.fuel_sunk(),
                    first_hour: ended.first.max(stretch.first),
                    last_hour: ended.last,
                    ended_by: ended.ended_by,
                });
            }
        }
        agreements.sort_by(|a, b| (a.last_hour, &a.terms.key).cmp(&(b.last_hour, &b.terms.key)));
        let mut warnings: Vec<String> = std::mem::take(&mut named.warnings).into_iter().collect();
        if partial > 0 {
            warnings.push(format!("{partial} ended agreements have unreadable or unrecorded fields at their last read; each missing field says why"));
        }
        let reply = LendLeaseHistoryReply {
            branch_id: params.branch_id.clone(),
            agreements,
            icons: named.icons(),
            variants: named.variants.into_values().collect(),
            warnings,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_columnar::{
        state::Kind,
        store::{Store, Value as StoreValue},
    };

    #[test]
    fn unavailable_numeric_and_pool_reads_stay_missing_with_reasons_never_zero() {
        let columns = Columns::read().unwrap();
        let kinds: Vec<Kind> = (0..TableId::LendLeaseAgreements.field_count())
            .map(|at| {
                if columns.pools.contains(&at) {
                    Kind::Opaque
                } else if at == columns.numbers[0] {
                    Kind::U32
                } else if at == columns.numbers[1] {
                    Kind::U64
                } else if at == columns.numbers[2] || at == columns.numbers[3] {
                    Kind::I32
                } else {
                    Kind::I64
                }
            })
            .collect();
        let mut store = Store::new(vec![kinds], vec![false]);
        store.begin_hour(0, &[]).unwrap();
        let key = u128::from(9001u32) << 64 | 9u128;
        let mut trailing_garbage = Vec::new();
        MarketPool::default().encode(&mut trailing_garbage).unwrap();
        trailing_garbage.push(0x7f);
        let slot = {
            let mut table = store.table(0).unwrap();
            let slot = table.insert(key).unwrap();
            table.put_value(slot, columns.pools[0], StoreValue::List(&[0xff, 0xff])).unwrap();
            table.put_value(slot, columns.pools[1], StoreValue::List(&trailing_garbage)).unwrap();
            slot
        };
        store.settle();
        let view = store.view(0);
        let (row, whole) = columns.row(&view, slot, key);
        assert!(!whole);
        assert_eq!(row.receiver, None);
        assert_eq!(row.revision, None);
        assert_eq!(row.active_since, None);
        assert_eq!(row.last_delivery, None);
        assert_eq!(row.fuel_daily_raw, None);
        assert_eq!(row.fuel_percentage_raw, None);
        assert_eq!(row.fuel_sent_raw, None);
        assert_eq!(row.fuel_sunk_raw, None);
        assert_eq!(row.last_fuel_delivered_raw, None);
        assert_eq!(row.monthly, Err(PoolMissing::Corrupt));
        assert_eq!(row.percentage, Err(PoolMissing::Corrupt));
        assert_eq!(row.once, Err(PoolMissing::Unrecorded));
        assert_eq!(row.delivered, Err(PoolMissing::Unrecorded));
        assert_eq!(row.sunk, Err(PoolMissing::Unrecorded));
        let mut called_with_zero = false;
        let terms = row.terms(|raw| {
            if raw == 0 {
                called_with_zero = true;
            }
            (raw == 9001).then(|| "GER".to_owned())
        }, &Named::default());
        assert!(!called_with_zero, "an unrecorded receiver never looks up country 0");
        assert_eq!(terms.giver.as_deref(), Some("GER"));
        assert_eq!(terms.receiver, None);
        assert!(terms.receiver_missing.is_some());
        assert_eq!(terms.revision.value_raw, None);
        assert!(terms.revision.missing.is_some());
        assert_eq!(terms.active_since.value, None);
        assert!(terms.active_since.missing.is_some());
        assert_eq!(terms.last_delivery.value, None);
        assert!(terms.last_delivery.missing.is_some());
        assert_eq!(terms.monthly.items, None);
        assert_eq!(terms.monthly.missing.as_deref(), Some("Could not be decoded at the agreement's last read"));
        assert_eq!(terms.percentage.items, None);
        assert_eq!(terms.percentage.missing.as_deref(), Some("Could not be decoded at the agreement's last read"));
        assert_eq!(terms.delivered.items, None);
        assert_eq!(terms.delivered.missing.as_deref(), Some("Not recorded at the agreement's last read"));
        assert_eq!(terms.fuel_daily_raw.value_raw, None);
        assert!(terms.fuel_daily_raw.missing.is_some());
        assert_eq!(terms.fuel_percentage_raw.value_raw, None);
        assert!(terms.fuel_percentage_raw.missing.is_some());
        assert_eq!(terms.last_fuel_delivered_raw.value_raw, None);
        assert!(terms.last_fuel_delivered_raw.missing.is_some());
        assert_eq!(row.fuel_sent().value_raw, None);
        assert!(row.fuel_sent().missing.is_some());
        assert_eq!(row.fuel_sunk().value_raw, None);
        assert!(row.fuel_sunk().missing.is_some());
    }
}
