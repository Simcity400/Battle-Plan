use super::{MarketItem, pool, pool_variants};
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
    fn reason(self) -> String {
        match self {
            Self::Unrecorded => "Not recorded at the agreement's last read".into(),
            Self::Corrupt => "Could not be decoded at the agreement's last read".into(),
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EndedLeaseTerms {
    pub key: String,
    #[doc = " The handle, hexadecimal."]
    pub id: String,
    pub giver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub giver_missing: Option<String>,
    pub receiver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub receiver_missing: Option<String>,
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub revision_missing: Option<String>,
    pub active_since: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub active_since_missing: Option<String>,
    pub last_delivery: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub last_delivery_missing: Option<String>,
    pub monthly: Option<Vec<MarketItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub monthly_missing: Option<String>,
    #[doc = " Production shares, /1000 percent."]
    pub percentage: Option<Vec<MarketItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub percentage_missing: Option<String>,
    pub once: Option<Vec<MarketItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub once_missing: Option<String>,
    pub delivered: Option<Vec<MarketItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub delivered_missing: Option<String>,
    pub sunk: Option<Vec<MarketItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sunk_missing: Option<String>,
    pub fuel_daily_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub fuel_daily_raw_missing: Option<String>,
    pub fuel_percentage_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub fuel_percentage_raw_missing: Option<String>,
    pub last_fuel_delivered_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub last_fuel_delivered_raw_missing: Option<String>,
}

impl EndedRow {
    fn key(&self) -> u128 {
        u128::from(self.country) << 64 | u128::from(self.id)
    }

    fn variants(&self) -> impl Iterator<Item = u64> + '_ {
        [&self.monthly, &self.percentage, &self.once, &self.delivered, &self.sunk]
            .into_iter()
            .filter_map(|held| held.as_ref().ok())
            .flat_map(|held| pool_variants(held))
    }

    fn terms(&self, mut tag: impl FnMut(u32) -> Option<String>, named: &Named) -> EndedLeaseTerms {
        let when = "at the agreement's last read";
        let amount_u64 = |value: Option<u64>, label: &str| -> (Option<String>, Option<String>) {
            match value {
                Some(value) => (Some(value.to_string()), None),
                None => (None, Some(format!("{label} not recorded {when}"))),
            }
        };
        let amount_i64 = |value: Option<i64>, label: &str| -> (Option<String>, Option<String>) {
            match value {
                Some(value) => (Some(value.to_string()), None),
                None => (None, Some(format!("{label} not recorded {when}"))),
            }
        };
        let hour = |value: Option<i32>, label: &str| -> (Option<i32>, Option<String>) {
            match value {
                Some(value) => (Some(value), None),
                None => (None, Some(format!("{label} not recorded {when}"))),
            }
        };
        let items = |held: &Result<SharedMarketPool, PoolMissing>| -> (Option<Vec<MarketItem>>, Option<String>) {
            match held {
                Ok(held) => (Some(pool(named, held)), None),
                Err(why) => (None, Some(why.reason())),
            }
        };
        let (giver, giver_missing) = match tag(self.country) {
            Some(name) => (Some(name), None),
            None => (None, Some(format!("Giver country {} had no tag {when}", self.country))),
        };
        let (receiver, receiver_missing) = match self.receiver {
            Some(raw) => match tag(raw) {
                Some(name) => (Some(name), None),
                None => (None, Some(format!("Receiver country {raw} had no tag {when}"))),
            },
            None => (None, Some(format!("Receiver not recorded {when}"))),
        };
        let (revision, revision_missing) = amount_u64(self.revision, "Revision");
        let (active_since, active_since_missing) = hour(self.active_since, "Start date");
        let (last_delivery, last_delivery_missing) = hour(self.last_delivery, "Last delivery date");
        let (monthly, monthly_missing) = items(&self.monthly);
        let (percentage, percentage_missing) = items(&self.percentage);
        let (once, once_missing) = items(&self.once);
        let (delivered, delivered_missing) = items(&self.delivered);
        let (sunk, sunk_missing) = items(&self.sunk);
        let (fuel_daily_raw, fuel_daily_raw_missing) = amount_i64(self.fuel_daily_raw, "Daily fuel");
        let (fuel_percentage_raw, fuel_percentage_raw_missing) = amount_i64(self.fuel_percentage_raw, "Fuel production share");
        let (last_fuel_delivered_raw, last_fuel_delivered_raw_missing) = amount_i64(self.last_fuel_delivered_raw, "Last fuel delivery");
        EndedLeaseTerms {
            key: self.key().to_string(),
            id: format!("{:016x}", self.id),
            giver,
            giver_missing,
            receiver,
            receiver_missing,
            revision,
            revision_missing,
            active_since,
            active_since_missing,
            last_delivery,
            last_delivery_missing,
            monthly,
            monthly_missing,
            percentage,
            percentage_missing,
            once,
            once_missing,
            delivered,
            delivered_missing,
            sunk,
            sunk_missing,
            fuel_daily_raw,
            fuel_daily_raw_missing,
            fuel_percentage_raw,
            fuel_percentage_raw_missing,
            last_fuel_delivered_raw,
            last_fuel_delivered_raw_missing,
        }
    }

    fn fuel_sent(&self) -> (Option<String>, Option<String>) {
        match self.fuel_sent_raw {
            Some(value) => (Some(value.to_string()), None),
            None => (None, Some("Fuel sent not recorded at the agreement's last read".into())),
        }
    }

    fn fuel_sunk(&self) -> (Option<String>, Option<String>) {
        match self.fuel_sunk_raw {
            Some(value) => (Some(value.to_string()), None),
            None => (None, Some("Fuel lost not recorded at the agreement's last read".into())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub row: EndedRow,
    pub whole: bool,
}

#[derive(Default)]
struct Runs {
    runs: HashMap<u32, Arc<Vec<Ended>>>,
}

struct Columns {
    numbers: [usize; 9],
    pools: [usize; 5],
}

impl Columns {
    fn read() -> io::Result<Self> {
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

    fn row(&self, view: &TableView<'_>, slot: u32, key: u128) -> (EndedRow, bool) {
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

pub fn ended_spans(history: &[(u128, Vec<lanes::Step>)], read: impl Fn(i32) -> bool) -> Vec<(u128, i32, i32)> {
    let mut out = Vec::new();
    for (key, steps) in history {
        let mut open: Option<i32> = None;
        for (at, step) in steps.iter().enumerate() {
            match (&step.cells, open) {
                (Some(_), None) => open = Some(step.hour),
                (None, Some(first)) => {
                    let back = steps[at + 1..].iter().any(|later| later.cells.is_some());
                    if read(step.hour) || !back {
                        out.push((*key, first, step.hour - 1));
                    }
                    open = None;
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
    let spans = ended_spans(&history.keys, |hour| reads.iter().any(|&(a, b)| a <= hour && hour <= b));
    let lives: HashMap<(i32, u128), i32> = spans.iter().map(|&(key, first, last)| ((last, key), first)).collect();
    let asks: Vec<(i32, u128)> = lives.keys().copied().collect();
    let mut ended = Vec::with_capacity(asks.len());
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, table, None, &asks, &mut |hour, key, row| {
            if let (Some((view, slot)), Some(&first)) = (row, lives.get(&(hour, key))) {
                let (row, whole) = columns.row(&view, slot, key);
                ended.push(Ended { key, first, last: hour, row, whole });
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
    pub terms: EndedLeaseTerms,
    pub fuel_sent_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub fuel_sent_raw_missing: Option<String>,
    pub fuel_sunk_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub fuel_sunk_raw_missing: Option<String>,
    #[doc = " The first hour it stood in its run, within the branch."]
    pub first_hour: i32,
    #[doc = " Its last read."]
    pub last_hour: i32,
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
        vec![EndedLeaseTerms::decl(), EndedLease::decl(), LendLeaseHistoryReply::decl()]
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
            let mine: Vec<&Ended> = ended.iter().filter(|ended| stretch.first <= ended.last && ended.last < stretch.last).collect();
            if mine.is_empty() {
                continue;
            }
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let variants: BTreeSet<u64> = mine.iter().flat_map(|ended| ended.row.variants()).collect();
            let variants: Vec<u64> = variants.into_iter().collect();
            names::name_at(recording, stretch.run, stretch.last, &variants, &[], &context, &mut named)?;
            for ended in mine {
                partial += usize::from(!ended.whole);
                let (fuel_sent_raw, fuel_sent_raw_missing) = ended.row.fuel_sent();
                let (fuel_sunk_raw, fuel_sunk_raw_missing) = ended.row.fuel_sunk();
                agreements.push(EndedLease {
                    terms: ended.row.terms(|raw| tags.tag_at(&links, raw, ended.last).map(str::to_owned), &named),
                    fuel_sent_raw,
                    fuel_sent_raw_missing,
                    fuel_sunk_raw,
                    fuel_sunk_raw_missing,
                    first_hour: ended.first.max(stretch.first),
                    last_hour: ended.last,
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
    use crate::products::economy::market::{LeaseTerms, lease_terms};
    use replay_columnar::{
        state::Kind,
        store::{Store, Value as StoreValue},
    };
    use replay_data::rows::LendLeaseAgreementRow;

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
        let terms = row.terms(
            |raw| {
                if raw == 0 {
                    called_with_zero = true;
                }
                (raw == 9001).then(|| "GER".to_owned())
            },
            &Named::default(),
        );
        assert!(!called_with_zero, "an unrecorded receiver never looks up country 0");
        assert_eq!(terms.giver.as_deref(), Some("GER"));
        assert_eq!(terms.giver_missing, None);
        assert_eq!(terms.receiver, None);
        assert_eq!(terms.receiver_missing.as_deref(), Some("Receiver not recorded at the agreement's last read"));
        assert_eq!(terms.revision, None);
        assert!(terms.revision_missing.is_some());
        assert_eq!(terms.active_since, None);
        assert!(terms.active_since_missing.is_some());
        assert_eq!(terms.last_delivery, None);
        assert!(terms.last_delivery_missing.is_some());
        assert_eq!(terms.monthly, None);
        assert_eq!(terms.monthly_missing.as_deref(), Some("Could not be decoded at the agreement's last read"));
        assert_eq!(terms.percentage, None);
        assert_eq!(terms.percentage_missing.as_deref(), Some("Could not be decoded at the agreement's last read"));
        assert_eq!(terms.delivered, None);
        assert_eq!(terms.delivered_missing.as_deref(), Some("Not recorded at the agreement's last read"));
        assert_eq!(terms.fuel_daily_raw, None);
        assert!(terms.fuel_daily_raw_missing.is_some());
        assert_eq!(terms.fuel_percentage_raw, None);
        assert!(terms.fuel_percentage_raw_missing.is_some());
        assert_eq!(terms.last_fuel_delivered_raw, None);
        assert!(terms.last_fuel_delivered_raw_missing.is_some());
        assert_eq!(row.fuel_sent().0, None);
        assert!(row.fuel_sent().1.is_some());
        assert_eq!(row.fuel_sunk().0, None);
        assert!(row.fuel_sunk().1.is_some());

        let untagged = EndedRow { receiver: Some(77), ..row };
        let untagged_terms = untagged.terms(|_| None, &Named::default());
        assert_eq!(untagged_terms.giver, None);
        assert_eq!(untagged_terms.giver_missing.as_deref(), Some("Giver country 9001 had no tag at the agreement's last read"));
        assert_eq!(untagged_terms.receiver, None);
        assert_eq!(untagged_terms.receiver_missing.as_deref(), Some("Receiver country 77 had no tag at the agreement's last read"));
    }

    #[test]
    fn fully_recorded_ended_row_serializes_byte_for_byte_as_main() {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct MainEndedLease {
            terms: LeaseTerms,
            fuel_sent_raw: String,
            fuel_sunk_raw: String,
            first_hour: i32,
            last_hour: i32,
        }

        let main_row = LendLeaseAgreementRow {
            country: 1,
            id: 1,
            receiver: 2,
            revision: 0,
            active_since: 100,
            last_delivery: 0,
            monthly: SharedMarketPool::default(),
            percentage: SharedMarketPool::default(),
            once: SharedMarketPool::default(),
            delivered: SharedMarketPool::default(),
            sunk: SharedMarketPool::default(),
            fuel_daily_raw: 0,
            fuel_percentage_raw: 0,
            fuel_sent_raw: 0,
            fuel_sunk_raw: 0,
            last_fuel_delivered_raw: 0,
        };
        let tag = |raw: u32| Some(if raw == 1 { "GER" } else { "FRA" }.to_owned());
        let main_lease = MainEndedLease {
            terms: lease_terms(&main_row, tag, &Named::default()),
            fuel_sent_raw: main_row.fuel_sent_raw.to_string(),
            fuel_sunk_raw: main_row.fuel_sunk_raw.to_string(),
            first_hour: 100,
            last_hour: 110,
        };
        let ended_row = EndedRow {
            country: main_row.country,
            id: main_row.id,
            receiver: Some(main_row.receiver),
            revision: Some(main_row.revision),
            active_since: Some(main_row.active_since),
            last_delivery: Some(main_row.last_delivery),
            monthly: Ok(main_row.monthly.clone()),
            percentage: Ok(main_row.percentage.clone()),
            once: Ok(main_row.once.clone()),
            delivered: Ok(main_row.delivered.clone()),
            sunk: Ok(main_row.sunk.clone()),
            fuel_daily_raw: Some(main_row.fuel_daily_raw),
            fuel_percentage_raw: Some(main_row.fuel_percentage_raw),
            fuel_sent_raw: Some(main_row.fuel_sent_raw),
            fuel_sunk_raw: Some(main_row.fuel_sunk_raw),
            last_fuel_delivered_raw: Some(main_row.last_fuel_delivered_raw),
        };
        let (fuel_sent_raw, fuel_sent_raw_missing) = ended_row.fuel_sent();
        let (fuel_sunk_raw, fuel_sunk_raw_missing) = ended_row.fuel_sunk();
        let current_lease = EndedLease {
            terms: ended_row.terms(tag, &Named::default()),
            fuel_sent_raw,
            fuel_sent_raw_missing,
            fuel_sunk_raw,
            fuel_sunk_raw_missing,
            first_hour: 100,
            last_hour: 110,
        };
        assert_eq!(
            serde_json::to_string(&current_lease).unwrap(),
            serde_json::to_string(&main_lease).unwrap(),
            "a fully recorded ended row serializes byte-for-byte identically to main"
        );
        let reply = LendLeaseHistoryReply {
            branch_id: "1".into(),
            agreements: vec![current_lease; 70_000],
            variants: Vec::new(),
            icons: Vec::new(),
            warnings: Vec::new(),
        };
        check_reply_budget(&reply).expect("70,000 fully recorded ended rows fit within the transport budget as on main");
    }
}
