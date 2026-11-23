use super::{LeaseTerms, lease_terms, lease_variants};
use dashboard_base::{Source, products::{BranchParams, LoadContext, Product, check_reply_budget}, queries::country_ids::Names};
use dashboard_world::products::equipment_names::{EquipmentIcon, VariantName, self as names, Named};
use replay_lanes::lanes::{self, Keys};
use replay_columnar::store::TableView;
use replay_data::{
    codec::Decoder,
    rows::{LendLeaseAgreementRow, MarketPool, SharedMarketPool, TableId},
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub row: LendLeaseAgreementRow,
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

    fn row(&self, view: &TableView<'_>, slot: u32, key: u128) -> (LendLeaseAgreementRow, bool) {
        let number = |at: usize| view.number(self.numbers[at], slot).unwrap_or_default();
        let mut whole = true;
        let mut pool = |at: usize| -> SharedMarketPool {
            match view.list(self.pools[at], slot).map(|bytes| MarketPool::decode(&mut Decoder::new(bytes))) {
                Some(Ok(pool)) => pool.into(),
                _ => {
                    whole = false;
                    SharedMarketPool::default()
                }
            }
        };
        let row = LendLeaseAgreementRow {
            country: (key >> 64) as u32,
            id: key as u64,
            receiver: number(0) as u32,
            revision: number(1) as u64,
            active_since: number(2) as i32,
            last_delivery: number(3) as i32,
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
    pub terms: LeaseTerms,
    pub fuel_sent_raw: String,
    pub fuel_sunk_raw: String,
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
            let mine: Vec<&Ended> = ended.iter().filter(|ended| stretch.first <= ended.last && ended.last < stretch.last).collect();
            if mine.is_empty() {
                continue;
            }
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let variants: BTreeSet<u64> = mine.iter().flat_map(|ended| lease_variants(&ended.row)).collect();
            let variants: Vec<u64> = variants.into_iter().collect();
            names::name_at(recording, stretch.run, stretch.last, &variants, &[], &context, &mut named)?;
            for ended in mine {
                partial += usize::from(!ended.whole);
                agreements.push(EndedLease {
                    terms: lease_terms(&ended.row, |raw| tags.tag_at(&links, raw, ended.last).map(str::to_owned), &named),
                    fuel_sent_raw: ended.row.fuel_sent_raw.to_string(),
                    fuel_sunk_raw: ended.row.fuel_sunk_raw.to_string(),
                    first_hour: ended.first.max(stretch.first),
                    last_hour: ended.last,
                });
            }
        }
        agreements.sort_by(|a, b| (a.last_hour, &a.terms.key).cmp(&(b.last_hour, &b.terms.key)));
        let mut warnings: Vec<String> = std::mem::take(&mut named.warnings).into_iter().collect();
        if partial > 0 {
            warnings.push(format!("{partial} ended agreements' schedules or deliveries could not be read at their last read; they show none"));
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
