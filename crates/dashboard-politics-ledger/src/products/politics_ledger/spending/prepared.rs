//! Prepared ledger observations use the production accounting and shared series.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::prepass::Extract;
use dashboard_base::prepared::priority::{JobPriority, Level};
use dashboard_base::prepared::series::{self, Accumulator, Reader, Run};
use replay::rows::RowCodec;

mod keys;

pub(crate) type PreparedContent = Result<Option<Arc<PoliticsContent>>, String>;

pub fn prepared_content(
    source: &Source,
    options: Option<&dashboard_core::config::ContentOptions>,
) -> PreparedContent {
    let content = options
        .map(|options| politics_content(source, options))
        .transpose()
        .map_err(|reason| reason.to_string())?;
    if let Some(content) = &content {
        let _ = content.prepare_power_content();
    }
    Ok(content)
}

#[derive(Default, Serialize, Deserialize)]
#[allow(clippy::type_complexity)]
pub struct Prepared {
    #[serde(with = "scan_state")]
    scan: Scanned,
    series: Accumulator,
    recurring: Accumulator,
    keys: keys::Keys,
    metadata: Catalogue,
    /// Only the newest native day remains mutable. Delayed settlements behind
    /// the published frontier use a sparse publication-hour event instead.
    tail: BTreeMap<i32, OpenDay>,
    frontier: Option<i32>,
    last_cover: BTreeMap<String, (bool, bool, Option<String>)>,
    identities: BTreeMap<String, (Option<u32>, Option<u32>, Option<String>)>,
    parts: BTreeMap<String, DayParts>,
    totals: BTreeMap<String, Totals>,
    open_notes: BTreeMap<String, BTreeMap<String, BTreeMap<String, u64>>>,
    last_capture: BTreeMap<String, LedgerCoverage>,
    last_presence: Option<i32>,
    #[serde(skip)]
    content: Option<Arc<PoliticsContent>>,
    #[serde(skip)]
    content_ready: bool,
    #[serde(skip)]
    current_daily_keys: BTreeSet<u128>,
    #[serde(skip)]
    scalar_values: BTreeMap<u64, Option<i128>>,
    #[serde(skip)]
    day_tags: BTreeMap<i32, BTreeSet<String>>,
    /// Whether every part not in `new_parts` names a capture of the last
    /// capture read.
    #[serde(skip)]
    parts_checked: bool,
    /// Parts added since the last capture read.
    #[serde(skip)]
    new_parts: BTreeSet<String>,
    /// The daily table allocation `current_daily_keys` were read from: an
    /// hour holding it holds the same rows.
    #[serde(skip)]
    daily_view: Option<replay::frame::ViewIdentity>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    /// The hours that only advanced captures, and those of the whole
    /// accounting.
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    paths: [usize; 2],
    /// Of the hours that only advanced captures, those a pre-pass worker's
    /// certificate advanced without joining their captures; and the hours
    /// whose countries were presented on several threads.
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    folds: [usize; 2],
    /// Take the whole accounting at every hour that is not quiet, so tests
    /// compare it with the advance path.
    #[cfg(test)]
    #[serde(skip)]
    dense: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Catalogue {
    first: Option<i32>,
    last: Option<i32>,
    native_through: Option<i32>,
    tail_ranges: BTreeMap<String, u64>,
    largest_lag: i32,
    items: BTreeMap<String, Item>,
    tooltips: BTreeMap<String, [Option<(i32, u64)>; 2]>,
    costs: BTreeMap<String, LedgerDailyCost>,
    categories: BTreeMap<String, BTreeMap<String, String>>,
    category_first: BTreeMap<String, i32>,
    actors: BTreeMap<String, BTreeSet<(u32, u32, String)>>,
    daily_lags: BTreeMap<String, [i32; 2]>,
    command_lag: i32,
    government_lag: i32,
    presentation: BTreeMap<String, BTreeMap<String, u64>>,
    pending_presentation: BTreeMap<String, BTreeMap<String, u64>>,
    formats: BTreeMap<String, BTreeSet<String>>,
    warnings: Vec<(Option<String>, BTreeMap<String, u64>)>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Item {
    tag: String,
    currency: String,
    item: LedgerSpendItem,
    category: LedgerSpendCategory,
    country_name: Option<String>,
    /// Production presentation visits mutations before daily components.
    first: [Option<(i32, u64, String)>; 2],
    facts: Payment,
    display: String,
    lag: i32,
}

#[derive(Default, Serialize, Deserialize)]
struct OpenDay {
    payments: BTreeMap<String, Payment>,
    daily: BTreeMap<String, LedgerDaily>,
    costs: BTreeMap<String, LedgerDailyCost>,
    governments: Vec<(String, LedgerGovernment)>,
}

#[derive(Default, Serialize, Deserialize)]
struct DayParts {
    day: i32,
    values: BTreeMap<String, Option<i128>>,
    labels: BTreeMap<String, String>,
    since: [Option<i32>; 2],
    last: Option<i32>,
    first: Option<i32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Event {
    Presence {
        row: PresenceRun,
    },
    Mutation {
        row: LedgerMutation,
        balance: Option<LedgerBalanceChange>,
        payment: Option<Payment>,
    },
    Cost {
        row: LedgerDailyCost,
        payment: Payment,
    },
    Government {
        row: LedgerGovernment,
    },
    Coverage {
        row: LedgerCoverage,
    },
    Hold {
        row: LedgerCommandHold,
    },
    Item {
        payment: Payment,
    },
    Late {
        payment: Payment,
        cost: Option<LedgerDailyCost>,
    },
    LateDaily {
        row: LedgerDaily,
    },
    Payment {
        payment: Payment,
    },
    RawCost {
        row: LedgerDailyCost,
    },
    Totals {
        payment: Payment,
        totals: Totals,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct Payment {
    spend: LedgerSpend,
    item: LedgerSpendItem,
    category: LedgerSpendCategory,
    ideas: Option<LedgerIdeaTooltips>,
    count: u32,
    known_raw: String,
    known_count: u32,
    identity: (u32, u32, String),
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Totals {
    #[serde(with = "decimal_i128")]
    known: i128,
    count: u64,
    rows: u64,
    unknown: u64,
    uncovered: u64,
    latest: Option<Payment>,
}

mod decimal_i128 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &i128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i128, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl Totals {
    fn add(&mut self, payment: &Payment) -> io::Result<()> {
        self.known = self
            .known
            .checked_add(amount(Some(&payment.known_raw))?.unwrap())
            .ok_or_else(|| dashboard_core::invalid("Politics subtotal overflow"))?;
        self.count = self
            .count
            .checked_add(u64::from(payment.spend.occurrences))
            .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))?;
        self.rows = self
            .rows
            .checked_add(u64::from(payment.count))
            .ok_or_else(|| dashboard_core::invalid("Politics row count overflow"))?;
        self.unknown = self
            .unknown
            .checked_add(u64::from(payment.spend.occurrences - payment.known_count))
            .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))?;
        self.uncovered = self
            .uncovered
            .checked_add(u64::from(!payment.spend.covered))
            .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))?;
        let later = self.latest.as_ref().is_none_or(|old| {
            (
                old.spend.hour,
                old.spend.sequence.parse::<u64>().ok(),
                old.spend.ordinal,
            ) <= (
                payment.spend.hour,
                payment.spend.sequence.parse::<u64>().ok(),
                payment.spend.ordinal,
            )
        });
        if later {
            self.latest = Some(payment.clone());
        }
        Ok(())
    }
}

fn is_recurring(category: &str) -> bool {
    matches!(
        category,
        "dailyCosts" | "organisationUpkeep" | "upkeep" | "dailyDeficit"
    )
}

/// Lookup operands shared by every payment of one country's presentation.
/// First occurrences preserve the presentation's ordered searches.
struct Payments<'a> {
    reply: &'a PoliticsLedgerReply,
    items: BTreeMap<&'a str, &'a LedgerSpendItem>,
    categories: BTreeMap<&'a str, &'a LedgerSpendCategory>,
    identities: BTreeMap<String, (u32, u32, String)>,
    order: BTreeMap<String, usize>,
    ideas: BTreeMap<Option<&'a str>, Option<LedgerIdeaTooltips>>,
    #[cfg(test)]
    indexed: usize,
}

impl<'a> Payments<'a> {
    fn new(reply: &'a PoliticsLedgerReply) -> Self {
        let mut result = Self {
            reply,
            items: BTreeMap::new(),
            categories: BTreeMap::new(),
            identities: BTreeMap::new(),
            order: BTreeMap::new(),
            ideas: BTreeMap::new(),
            #[cfg(test)]
            indexed: 0,
        };
        if reply.spending.is_empty() {
            return result;
        }
        for item in &reply.spend_items {
            #[cfg(test)]
            {
                result.indexed += 1;
            }
            result.items.entry(&item.id).or_insert(item);
        }
        for category in &reply.spend_categories {
            #[cfg(test)]
            {
                result.indexed += 1;
            }
            result.categories.entry(&category.id).or_insert(category);
        }
        for (order, row) in reply.mutations.iter().enumerate() {
            #[cfg(test)]
            {
                result.indexed += 1;
            }
            let id = mutation_id(row);
            result
                .identities
                .entry(id.clone())
                .or_insert_with(|| (row.country, row.instance, row.epoch.clone()));
            result.order.entry(id).or_insert(order);
        }
        for (order, row) in reply.daily_costs.iter().enumerate() {
            #[cfg(test)]
            {
                result.indexed += 1;
            }
            let id = format!(
                "daily:{}:{}:{}:{}:{}",
                row.country, row.instance, row.epoch, row.day_hour, row.ordinal
            );
            result
                .identities
                .entry(id.clone())
                .or_insert_with(|| (row.country, row.instance, row.epoch.clone()));
            result.order.entry(id).or_insert(order);
        }
        result
    }

    fn payment(&mut self, spend: &LedgerSpend) -> io::Result<Payment> {
        let item = *self
            .items
            .get(spend.item_id.as_str())
            .ok_or_else(|| dashboard_core::invalid("Missing prepared spending item"))?;
        let ideas = self
            .ideas
            .entry(item.tooltip_token.as_deref())
            .or_insert_with(|| {
                self.reply.idea_tooltips.as_ref().map(|ideas| {
                    let infos: Vec<_> = ideas
                        .ideas
                        .iter()
                        .filter(|info| Some(&info.token) == item.tooltip_token.as_ref())
                        .cloned()
                        .collect();
                    let (modifiers, texts) = idea_history::effect_keys(&infos);
                    LedgerIdeaTooltips {
                        ideas: infos,
                        artwork: ideas
                            .artwork
                            .iter()
                            .filter(|art| Some(&art.token) == item.tooltip_token.as_ref())
                            .cloned()
                            .collect(),
                        modifiers: ideas
                            .modifiers
                            .iter()
                            .filter(|modifier| modifiers.contains(&modifier.key))
                            .cloned()
                            .collect(),
                        texts: ideas
                            .texts
                            .iter()
                            .filter(|text| texts.contains(&text.key))
                            .cloned()
                            .collect(),
                    }
                })
            })
            .clone();
        let identity = self
            .identities
            .get(&spend.id)
            .cloned()
            .ok_or_else(|| dashboard_core::invalid("Missing prepared payment identity"))?;
        Ok(Payment {
            spend: spend.clone(),
            item: item.clone(),
            category: (*self
                .categories
                .get(spend.category.as_str())
                .ok_or_else(|| dashboard_core::invalid("Missing prepared spending category"))?)
            .clone(),
            ideas,
            count: 1,
            known_raw: spend.charge_raw.clone().unwrap_or_else(|| "0".into()),
            identity,
            known_count: if spend.charge_raw.is_some() {
                spend.occurrences
            } else {
                0
            },
        })
    }
}

fn raw_cost_field(cost: &LedgerDailyCost) -> io::Result<String> {
    Ok(format!(
        "raw-cost:{}",
        serde_json::to_string(&(
            cost.country,
            cost.instance,
            &cost.epoch,
            cost.ordinal,
            &cost.component,
            &cost.cause,
            &cost.item
        ))?
    ))
}
fn daily_prefix(identity: &(u32, u32, String)) -> io::Result<String> {
    Ok(format!("daily:{}", serde_json::to_string(identity)?))
}
fn daily_id(tag: &str, row: &LedgerDaily) -> io::Result<String> {
    Ok(series::field_key(
        tag,
        &serde_json::to_string(&(row.country, row.instance, &row.epoch))?,
    ))
}
fn display_field(payment: &Payment) -> io::Result<String> {
    Ok(serde_json::to_string(&(
        &payment.spend.currency,
        &payment.item.group_id,
        &payment.item.tooltip_token,
        &payment.item.action,
        &payment.item.icon,
        &payment.item.unknown,
    ))?)
}
fn item_field(payment: &Payment) -> io::Result<String> {
    Ok(serde_json::to_string(&(
        &payment.identity,
        display_field(payment)?,
    ))?)
}

/// An identity's open parts, noting one added since the last capture read.
fn part<'a>(
    parts: &'a mut BTreeMap<String, DayParts>,
    added: &mut BTreeSet<String>,
    identity: String,
) -> &'a mut DayParts {
    if !parts.contains_key(&identity) {
        added.insert(identity.clone());
    }
    parts.entry(identity).or_default()
}

fn amount(value: Option<&str>) -> io::Result<Option<i128>> {
    value
        .map(str::parse)
        .transpose()
        .map_err(|error: std::num::ParseIntError| dashboard_core::invalid(error.to_string()))
}
fn add_raw(left: &mut Option<String>, right: Option<&str>) -> io::Result<()> {
    *left = match (amount(left.as_deref())?, amount(right)?) {
        (Some(left), Some(right)) => Some(
            left.checked_add(right)
                .ok_or_else(|| dashboard_core::invalid("Politics subtotal overflow"))?
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
        .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))?;
    left.spend.covered &= right.spend.covered;
    if left.spend.category != "upkeep" {
        left.count = left
            .count
            .checked_add(right.count)
            .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))?;
    }
    left.known_count = left
        .known_count
        .checked_add(right.known_count)
        .ok_or_else(|| dashboard_core::invalid("Politics payment count overflow"))?;
    left.known_raw = amount(Some(&left.known_raw))?
        .unwrap()
        .checked_add(amount(Some(&right.known_raw))?.unwrap())
        .ok_or_else(|| dashboard_core::invalid("Politics subtotal overflow"))?
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

impl Prepared {
    #[cfg(test)]
    fn measured_work(&self) -> dashboard_base::prepared::observer_tests::Work {
        let mut work = self.work;
        work.row_visits = self.scan.inputs.row_visits;
        work
    }
    pub fn resume(&mut self, frame: &DecodedFrame) {
        self.scan.resume(frame);
        // Rebuild the current day's clock operands, never completed facts.
        if let Some(hour) = frame
            .hour
            .div_euclid(24)
            .checked_add(1)
            .and_then(|day| day.checked_mul(24))
        {
            for (tag, country) in &self.scan.countries {
                let daily = country
                    .daily
                    .iter()
                    .any(|row| row.row.day_hour == frame.hour.div_euclid(24) * 24);
                let charged = country
                    .coverage
                    .last()
                    .and_then(|row| row.country)
                    .and_then(|raw| self.parts.get(&raw.to_string()))
                    .is_some_and(|part| !part.values.is_empty());
                if daily || charged {
                    self.day_tags.entry(hour).or_default().insert(tag.clone());
                }
            }
        }
    }
    pub fn bytes(&self) -> usize {
        self.series.bytes() + self.recurring.bytes()
    }
    /// The hours observed that only advanced captures, without the whole
    /// accounting.
    #[cfg(any(test, feature = "test-support"))]
    pub fn advanced_hours(&self) -> usize {
        self.paths[0]
    }
    /// The hours observed with the whole accounting.
    #[cfg(any(test, feature = "test-support"))]
    pub fn accounted_hours(&self) -> usize {
        self.paths[1]
    }
    /// The hours a pre-pass worker's certificate advanced without joining
    /// their captures ([`Self::observe_folded`]).
    #[cfg(any(test, feature = "test-support"))]
    pub fn certified_hours(&self) -> usize {
        self.folds[0]
    }
    /// The hours whose countries were presented on several threads.
    #[cfg(any(test, feature = "test-support"))]
    pub fn parallel_hours(&self) -> usize {
        self.folds[1]
    }
    /// Whether every hour that is not quiet takes the whole accounting.
    fn dense(&self) -> bool {
        #[cfg(test)]
        {
            self.dense
        }
        #[cfg(not(test))]
        {
            false
        }
    }
    fn key(&mut self, tag: &str, field: &str) -> io::Result<u64> {
        self.keys.key(tag, field)
    }
    fn scalar(
        &mut self,
        tag: &str,
        field: &str,
        hour: i32,
        value: Option<i128>,
        starts: bool,
        boundary: bool,
    ) -> io::Result<()> {
        let key = self.key(tag, field)?;
        if starts || boundary || self.scalar_values.get(&key) != Some(&value) {
            self.series
                .observe_boundary(key, hour, value, starts, boundary);
            self.scalar_values.insert(key, value);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.emitted += 1;
            }
        }
        Ok(())
    }
    fn native(&mut self, tag: &str, field: &str, day: i32, value: Option<i128>) -> io::Result<()> {
        let key = self.key(tag, field)?;
        if let Some(value) = value {
            self.recurring
                .push(key, Run::new(day, day.saturating_add(23), value, false));
        }
        Ok(())
    }
    /// The recording this branch is prepared from. A row the next read dates
    /// at a previous read is described from that read decoded again, so only
    /// the previous read's naming tables are held between hours.
    pub fn decode_previous_from(&mut self, replay: Arc<replay::container::Replay>) {
        self.scan.replay = Some(replay);
    }
    pub fn configure(&mut self, content: &PreparedContent) {
        if self.content_ready {
            return;
        }
        self.content_ready = true;
        self.content = match content {
            Ok(content) => content.clone(),
            Err(reason) => {
                // Keep the oracle's refusal once for this branch, including
                // across resume; unavailable content does not stop other families.
                self.scan
                    .warnings
                    .entry(None)
                    .or_default()
                    .entry(reason.to_string())
                    .or_insert(1);
                None
            }
        };
    }
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        cancelled_request(cancelled)?;
        let content = self.content.clone();
        self.observe_changed_frame(frame, content.as_deref(), changes, cancelled)
    }

    /// [`Self::observe`] of an hour of a block a pre-pass worker read
    /// ([`BlockFacts`]), with what it folded of the block so far (`fold`).
    /// The block's first hour is observed; when the join it leaves is the
    /// worker's after that hour, every later hour the worker certified (its
    /// changed captures only extend to it and nothing else joins a country)
    /// that only advances captures here too
    /// ([`Scanned::certified_advance`]) takes the advance path without
    /// joining its captures, which the certificate names
    /// ([`Self::observe_certified`]). Every other hour is observed. Both
    /// publish and keep what [`Self::observe`] does.
    pub fn observe_folded(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
        fold: &mut BlockFold,
    ) -> io::Result<()> {
        cancelled_request(cancelled)?;
        let content = self.content.clone();
        let Some(joined) = fold.joined else {
            let result = self.observe_changed_frame(frame, content.as_deref(), changes, cancelled);
            fold.joined = Some(
                result.is_ok()
                    && fold.facts.first.as_ref().is_some_and(|(hour, inputs)| {
                        *hour == frame.hour && self.scan.inputs.same_join(inputs)
                    }),
            );
            return result;
        };
        if joined
            && let Some(advanced) = fold.certified(frame.hour)
            && !self.dense()
            && self.scan.certified_advance(
                frame,
                changes,
                &advanced,
                self.day_tags
                    .first_key_value()
                    .is_some_and(|(hour, _)| *hour <= frame.hour),
            )
        {
            return self.observe_certified(frame, content.as_deref(), changes, &advanced);
        }
        self.observe_changed_frame(frame, content.as_deref(), changes, cancelled)
    }

    /// [`Self::observe_changed_frame`] of an hour
    /// [`Scanned::certified_advance`] accepted, in its order: the hour is
    /// accepted, its join only extends `advanced` (`Scanned::inputs_advanced`),
    /// no capture is due and no day boundary, so it takes the advance path
    /// ([`Self::observe_advance`]).
    fn observe_certified(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
        changes: &FrameChanges,
        advanced: &Arc<[(u32, i32)]>,
    ) -> io::Result<()> {
        self.scan.advance(frame.hour)?;
        self.series.advance(frame.hour)?;
        // The hour continues the last: every held value stays observed.
        self.metadata.first.get_or_insert(frame.hour);
        self.metadata.last = Some(frame.hour);
        self.scan.inputs_advanced(frame, changes, advanced);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.paths[0] += 1;
            self.folds[0] += 1;
        }
        self.observe_advance(frame, content, changes)
    }

    /// After a block observed with [`Self::observe_folded`]: the captures it
    /// advanced reach the last hour they did.
    pub fn end_block(&mut self) {
        self.scan.materialize_advance();
    }

    fn register(
        &mut self,
        tag: &str,
        projected: &Payment,
        hour: i32,
        order: u64,
        cost: bool,
    ) -> io::Result<String> {
        let field = item_field(projected)?;
        let id = series::field_key(tag, &field);
        let kind = usize::from(cost);
        let mut identity = projected.item.clone();
        identity.id.clear();
        let changed = self.metadata.items.get(&id).is_none_or(|old| {
            let mut previous = old.item.clone();
            previous.id.clear();
            previous != identity
                || old.category != projected.category
                || old.facts.ideas != projected.ideas
                || old.country_name != projected.spend.country_name
        });
        let item = self.metadata.items.entry(id).or_insert_with(|| Item {
            tag: tag.into(),
            currency: projected.spend.currency.clone(),
            item: projected.item.clone(),
            category: projected.category.clone(),
            country_name: projected.spend.country_name.clone(),
            first: [None, None],
            facts: projected.clone(),
            display: display_field(projected).expect("display identity serializes"),
            lag: 0,
        });
        item.first[kind].get_or_insert((hour, order, projected.spend.item_id.clone()));
        item.country_name = projected.spend.country_name.clone();
        if !is_recurring(&projected.spend.category)
            || self
                .frontier
                .is_none_or(|frontier| projected.spend.hour.div_euclid(24) * 24 > frontier)
        {
            item.lag = item.lag.max(projected.spend.read_hour.saturating_sub(
                if is_recurring(&projected.spend.category) {
                    projected.spend.hour.div_euclid(24) * 24
                } else {
                    projected.spend.hour
                },
            ));
        }
        // IDs are local to the frame builder; compare facts with their stable identity.
        item.item = projected.item.clone();
        item.facts = projected.clone();
        if changed {
            let key = self.key(tag, &format!("{field}:facts"))?;
            self.series.event(
                key,
                hour,
                &Event::Item {
                    payment: projected.clone(),
                },
            )?;
        }
        Ok(field)
    }

    fn recurring_payment(
        &mut self,
        tag: &str,
        mut projected: Payment,
        cost: Option<LedgerDailyCost>,
    ) -> io::Result<()> {
        let native_day = projected.spend.hour.div_euclid(24) * 24;
        projected.spend.changes.clear();
        if self.frontier.is_some_and(|frontier| native_day <= frontier) {
            let key = self.key(tag, "late-recurring")?;
            self.series.event(
                key,
                projected.spend.read_hour,
                &Event::Late {
                    payment: projected,
                    cost,
                },
            )?;
        } else {
            self.metadata.largest_lag = self
                .metadata
                .largest_lag
                .max(projected.spend.read_hour.saturating_sub(native_day));
            if projected.spend.currency == "Command" {
                self.metadata.command_lag = self
                    .metadata
                    .command_lag
                    .max(projected.spend.read_hour.saturating_sub(native_day));
            }

            let id = series::field_key(tag, &item_field(&projected)?);
            let payments = &mut self.tail.entry(native_day).or_default().payments;
            match payments.entry(id) {
                std::collections::btree_map::Entry::Occupied(mut old) => {
                    merge_payment(old.get_mut(), projected)?
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(projected);
                }
            }
            if let Some(cost) = cost {
                let field = raw_cost_field(&cost)?;
                self.tail
                    .entry(native_day)
                    .or_default()
                    .costs
                    .insert(series::field_key(tag, &field), cost);
            }
        }
        Ok(())
    }

    fn settle_before(&mut self, newest: i32) -> io::Result<()> {
        while self
            .tail
            .first_key_value()
            .is_some_and(|(day, _)| *day < newest)
        {
            let (day, mut open) = self.tail.pop_first().unwrap();
            // Every key for this day is written before advancing the shared axis.
            let mut categories = BTreeMap::<(String, String, String), i32>::new();
            for projected in open.payments.values() {
                let tag = &projected.spend.country_tag;
                let field = item_field(projected)?;
                categories
                    .entry((
                        tag.clone(),
                        projected.spend.currency.clone(),
                        projected.spend.category.clone(),
                    ))
                    .and_modify(|read| *read = (*read).min(projected.spend.read_hour))
                    .or_insert(projected.spend.read_hour);
                for (suffix, value) in [
                    ("charge", amount(Some(&projected.known_raw))?),
                    (
                        "realized-positive",
                        amount(projected.spend.realized_delta_raw.as_deref())?
                            .map(|value| value.max(0)),
                    ),
                    (
                        "realized-negative",
                        amount(projected.spend.realized_delta_raw.as_deref())?
                            .map(|value| value.min(0)),
                    ),
                    (
                        "realized-unknown",
                        Some(i128::from(projected.spend.realized_delta_raw.is_none())),
                    ),
                    ("count", Some(i128::from(projected.spend.occurrences))),
                    ("rows", Some(i128::from(projected.count))),
                    ("presence", Some(1)),
                    ("uncovered", Some(i128::from(!projected.spend.covered))),
                    (
                        "unknown",
                        Some(i128::from(
                            projected.spend.occurrences - projected.known_count,
                        )),
                    ),
                    (
                        "lag",
                        Some(i128::from(projected.spend.read_hour) - i128::from(day)),
                    ),
                    (
                        "hour",
                        Some(i128::from(projected.spend.hour.rem_euclid(24))),
                    ),
                    ("sequence", amount(Some(&projected.spend.sequence))?),
                    ("ordinal", Some(i128::from(projected.spend.ordinal))),
                ] {
                    self.native(tag, &format!("{field}:{suffix}"), day, value)?;
                }
            }
            for ((tag, currency, category), read) in categories {
                self.native(
                    &tag,
                    &format!("category:{currency}:{category}:days"),
                    day,
                    Some(1),
                )?;
                self.native(
                    &tag,
                    &format!("category:{currency}:{category}:lag"),
                    day,
                    Some(i128::from(read) - i128::from(day)),
                )?;
            }
            for (id, row) in open.daily {
                let (tag, _): (String, String) = serde_json::from_str(&id)?;
                self.publish_daily(&tag, &row)?;
            }
            for (id, row) in open.costs {
                let (tag, field): (String, String) = serde_json::from_str(&id)?;
                for (suffix, value) in [
                    ("amount", amount(Some(&row.amount_raw))?),
                    ("sequence", amount(Some(&row.pp_sequence))?),
                    ("lag", Some(i128::from(row.read_hour) - i128::from(day))),
                    ("covered", Some(i128::from(row.covered))),
                ] {
                    self.native(&tag, &format!("{field}:{suffix}"), day, value)?;
                }
            }
            let mut government_lags = BTreeMap::<(String, i32), i32>::new();
            open.governments.sort_by_key(|(tag, row)| {
                (tag.clone(), row.hour, row.sequence.parse::<u64>().ok())
            });
            for (tag, row) in open.governments {
                government_lags
                    .entry((tag.clone(), row.hour))
                    .and_modify(|lag| *lag = (*lag).max(row.read_hour.saturating_sub(row.hour)))
                    .or_insert(row.read_hour.saturating_sub(row.hour));
                let key = self.key(&tag, "governments")?;
                self.recurring
                    .event(key, row.hour, &Event::Government { row })?;
            }
            for ((tag, hour), lag) in government_lags {
                let key = self.key(&tag, "governments:lag")?;
                // Each native event hour is a keyboard stop, even when adjacent
                // hours have equal supplying lags.
                self.recurring.push_boundary(
                    key,
                    Run::new(hour, hour, i128::from(lag), false),
                    true,
                );
            }
            self.recurring.through(day.saturating_add(23));
            self.frontier = Some(day);
            self.metadata.native_through = Some(day.saturating_add(23));
        }
        Ok(())
    }

    fn publish_daily(&mut self, tag: &str, row: &LedgerDaily) -> io::Result<()> {
        let day = row.day_hour;
        let prefix = daily_prefix(&(row.country, row.instance, row.epoch.clone()))?;
        for (field, value) in daily_values(row)? {
            self.native(tag, &format!("{prefix}:{field}"), day, value)?;
        }
        for (currency, gain, net, applied) in [
            (
                "Political",
                amount(row.pp_gross_raw.as_deref())?,
                amount(row.pp_income_raw.as_deref())?,
                amount(row.pp_realized_raw.as_deref())?,
            ),
            (
                "Command",
                amount(row.cp_income_raw.as_deref())?,
                amount(row.cp_income_raw.as_deref())?,
                amount(row.cp_realized_raw.as_deref())?,
            ),
        ] {
            let gained = gain.map(|gain| {
                if currency == "Command" {
                    gain.max(0)
                } else {
                    gain
                }
            });
            let lost = net.zip(applied).map(|(net, applied)| {
                if net > 0 && applied < net {
                    net - applied.max(0)
                } else {
                    0
                }
            });
            self.native(
                tag,
                &format!("{prefix}:{currency}:gained-positive"),
                day,
                gained.map(|value| value.max(0)),
            )?;
            self.native(
                tag,
                &format!("{prefix}:{currency}:gained-negative"),
                day,
                gained.map(|value| value.min(0)),
            )?;
            self.native(tag, &format!("{prefix}:{currency}:lost"), day, lost)?;
            self.native(
                tag,
                &format!("{prefix}:{currency}:incomplete"),
                day,
                Some(i128::from(gain.is_none())),
            )?;
            let read = if currency == "Political" {
                row.pp_read_hour
            } else {
                row.cp_read_hour
            };
            self.native(
                tag,
                &format!("{prefix}:{currency}:days"),
                day,
                Some(i128::from(read.is_some())),
            )?;
            self.native(
                tag,
                &format!("{prefix}:{currency}:availability-lag"),
                day,
                Some(i128::from(read.unwrap_or(row.read_hour)) - i128::from(day)),
            )?;
        }
        Ok(())
    }

    fn publish_payment(&mut self, tag: &str, hour: i32, payment: &Payment) -> io::Result<()> {
        let id = series::field_key(tag, &item_field(payment)?);
        let totals = self.totals.entry(id).or_default();
        totals.add(payment)?;
        let totals = totals.clone();
        let key = self.key(tag, &format!("{}:totals", item_field(payment)?))?;
        self.series.event(
            key,
            hour,
            &Event::Totals {
                payment: payment.clone(),
                totals,
            },
        )?;
        Ok(())
    }

    fn record_parts(&mut self, reply: &PoliticsLedgerReply, hour: i32) -> io::Result<()> {
        let day = hour.div_euclid(24) * 24;
        let mut identities = BTreeMap::new();
        for row in &reply.mutations {
            identities.insert(
                mutation_id(row),
                (row.country, row.instance, row.epoch.clone()),
            );
        }
        for row in &reply.daily_costs {
            identities.insert(
                format!(
                    "daily:{}:{}:{}:{}:{}",
                    row.country, row.instance, row.epoch, row.day_hour, row.ordinal
                ),
                (row.country, row.instance, row.epoch.clone()),
            );
        }
        for spend in &reply.spending {
            if spend.category == "dailyDeficit" {
                continue;
            }
            let Some(identity) = identities.get(&spend.id) else {
                return Err(dashboard_core::invalid("Missing prepared payment identity"));
            };
            let open = part(
                &mut self.parts,
                &mut self.new_parts,
                serde_json::to_string(&identity.0)?,
            );
            if open.day != day {
                open.day = day;
                open.values.clear();
            }
            let category = format!("{}:{}", spend.currency, spend.category);
            if let Some(info) = reply
                .spend_categories
                .iter()
                .find(|item| item.id == spend.category)
            {
                open.labels.insert(category.clone(), info.name.clone());
            }
            for change in &spend.changes {
                if change
                    .hour
                    .is_none_or(|hour| hour < day || hour > day.saturating_add(23))
                {
                    continue;
                }
                let value = open.values.entry(category.clone()).or_insert(Some(0));
                *value = sum_raw([*value, amount(change.charge_raw.as_deref())?].into_iter())?;
            }
        }
        Ok(())
    }

    fn project_parts(
        &mut self,
        tag: &str,
        hour: i32,
        identity: Option<(u32, u32, String)>,
        starts: bool,
    ) -> io::Result<()> {
        let day = hour.div_euclid(24) * 24;
        let country_first = identity
            .as_ref()
            .map(|identity| -> io::Result<i32> {
                let open = part(
                    &mut self.parts,
                    &mut self.new_parts,
                    serde_json::to_string(&identity.0)?,
                );
                if !starts
                    && self
                        .scan
                        .clock
                        .previous
                        .and_then(|last| last.checked_add(1))
                        == Some(hour)
                    && open.last.is_some()
                {
                    open.last = self.scan.clock.previous;
                }
                if open.last.and_then(|last| last.checked_add(1)) != Some(hour) {
                    open.first = Some(hour);
                }
                Ok(*open.first.get_or_insert(hour))
            })
            .transpose()?;
        let prefix = format!("[{},", serde_json::to_string(tag)?);
        let observations: Vec<_> = self
            .tail
            .get(&day)
            .into_iter()
            .flat_map(|open| {
                open.daily
                    .range(prefix.clone()..)
                    .take_while(|(key, _)| key.starts_with(&prefix))
                    .map(|(_, row)| row)
            })
            .filter(|row| {
                country_first.is_some_and(|first| first <= day)
                    && identity
                        .as_ref()
                        .is_some_and(|identity| row.country == identity.0)
            })
            .collect();
        let supplied = |command: bool| {
            let mut rows = observations.iter().filter(|row| {
                if command {
                    row.cp_read_hour
                } else {
                    row.pp_read_hour
                }
                .is_some_and(|read| read <= hour)
            });
            let first = rows.next().copied();
            if rows.next().is_some() { None } else { first }
        };
        let current = supplied(false);
        let gross = current
            .map(|row| amount(row.pp_gross_raw.as_deref()))
            .transpose()?
            .flatten();
        let cost = current
            .map(|row| amount(row.pp_cost_raw.as_deref()))
            .transpose()?
            .flatten();
        let net = current
            .map(|row| amount(row.pp_income_raw.as_deref()))
            .transpose()?
            .flatten();
        let (gross, cost, net) = if gross.is_some() && cost.is_some() && net.is_some() {
            (gross, cost, net)
        } else {
            (None, None, None)
        };
        let cp = supplied(true)
            .map(|row| amount(row.cp_income_raw.as_deref()))
            .transpose()?
            .flatten();
        for (field, value) in [
            ("parts:Political:income", gross),
            ("parts:Political:costs", cost.map(|value| -value)),
            (
                "parts:Political:wrapping",
                gross
                    .zip(cost)
                    .zip(net)
                    .map(|((gross, cost), net)| net - (gross - cost)),
            ),
            ("parts:Political:income-total", net),
            ("parts:Command:income", cp),
            ("parts:Command:income-total", cp),
        ] {
            self.scalar(tag, field, hour, value, starts, false)?;
        }
        let identity_key = identity
            .as_ref()
            .map(|identity| serde_json::to_string(&identity.0))
            .transpose()?;
        let known = self
            .last_cover
            .get(tag)
            .map(|(pp, cp, _)| [*pp, *cp])
            .unwrap_or([false; 2]);
        let (values, labels, coverage) = if let Some(identity_key) = identity_key {
            let open = part(&mut self.parts, &mut self.new_parts, identity_key);
            if open.day != day {
                open.day = day;
                open.values.clear();
            }
            for (index, known) in known.into_iter().enumerate() {
                open.since[index] = if known {
                    Some(
                        if open.last.and_then(|hour| hour.checked_add(1)) == Some(hour) {
                            open.since[index].unwrap_or(hour)
                        } else {
                            hour
                        },
                    )
                } else {
                    None
                };
            }
            open.last = Some(hour);
            if !open.values.is_empty()
                && let Some(next) = day.checked_add(24)
            {
                self.day_tags
                    .entry(next)
                    .or_default()
                    .insert(tag.to_owned());
            }
            (open.values.clone(), open.labels.clone(), open.since)
        } else {
            (BTreeMap::new(), BTreeMap::new(), [None; 2])
        };
        self.metadata
            .categories
            .entry(tag.to_owned())
            .or_default()
            .extend(labels);
        let categories = self
            .metadata
            .categories
            .get(tag)
            .cloned()
            .unwrap_or_default();
        for currency in ["Political", "Command"] {
            let covered =
                coverage[usize::from(currency == "Command")].is_some_and(|first| first <= day);
            let total = if covered {
                sum_raw(
                    categories
                        .keys()
                        .filter(|category| category.starts_with(&format!("{currency}:")))
                        .map(|category| values.get(category).copied().unwrap_or(Some(0))),
                )?
            } else {
                None
            };
            self.scalar(
                tag,
                &format!("parts:{currency}:zero"),
                hour,
                (covered && total.is_some()).then_some(0),
                starts,
                false,
            )?;
            for category in categories
                .keys()
                .filter(|category| category.starts_with(&format!("{currency}:")))
            {
                self.metadata
                    .category_first
                    .entry(series::field_key(tag, category))
                    .or_insert(hour);
                let value = if covered && total.is_some() {
                    values.get(category).copied().unwrap_or(Some(0))
                } else {
                    None
                };
                self.scalar(
                    tag,
                    &format!("parts:{category}"),
                    hour,
                    value,
                    starts,
                    false,
                )?;
            }
            self.scalar(
                tag,
                &format!("parts:{currency}:spending-total"),
                hour,
                total,
                starts,
                false,
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn observe_frame(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.observe_changed_frame(
            frame,
            content,
            &FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [false; replay::frame::TABLE_COUNT],
                slot_read: [false; replay::frame::TABLE_COUNT],
                updates: None,
            },
            cancelled,
        )
    }

    fn observe_changed_frame(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.advance(frame.hour)?;
        self.series.advance(frame.hour)?;
        if self
            .scan
            .clock
            .previous
            .and_then(|hour| hour.checked_add(1))
            != Some(frame.hour)
        {
            // advance closes active runs at a recording gap. Equal values in
            // the new accepted episode must be observed again to become active.
            self.scalar_values.clear();
        }
        self.metadata.first.get_or_insert(frame.hour);
        self.metadata.last = Some(frame.hour);
        let changed = self.scan.inputs_changed(frame, changes);
        let expires = self.scan.clock.next.is_some_and(|hour| hour <= frame.hour);
        let mut daily_tags = BTreeSet::new();
        while self
            .day_tags
            .first_key_value()
            .is_some_and(|(hour, _)| *hour <= frame.hour)
        {
            daily_tags.extend(self.day_tags.pop_first().unwrap().1);
        }
        if !changed && !expires && daily_tags.is_empty() && self.scan.clock.stable {
            self.scan.retain_previous(frame);
            return Ok(());
        }
        let expired: BTreeSet<_> = if expires {
            self.scan
                .inputs
                .deadlines
                .range(self.scan.clock.next.unwrap()..=frame.hour)
                .flat_map(|(_, raws)| raws)
                .filter_map(|raw| self.scan.inputs.links.get(raw))
                .copied()
                .collect()
        } else {
            BTreeSet::new()
        };
        // Every capture extended to the hour and nothing else read changes
        // which country the accounting visits: it would visit none.
        if expired.is_empty()
            && daily_tags.is_empty()
            && !self.dense()
            && self.scan.advances_only(frame)
        {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.paths[0] += 1;
            }
            return self.observe_advance(frame, content, changes);
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.paths[1] += 1;
        }
        if expires {
            let inputs = Arc::make_mut(&mut self.scan.inputs);
            inputs.slots.extend(expired);
        }
        self.scan.dirty.extend(daily_tags);
        let hold_boundaries: BTreeSet<_> = if self.scan.inputs.countries_changed {
            let current = self.scan.inputs.tag_slots.keys().cloned().collect();
            self.scan
                .clock
                .active
                .symmetric_difference(&current)
                .cloned()
                .collect()
        } else {
            BTreeSet::new()
        };
        self.scan.dirty.extend(hold_boundaries.iter().cloned());
        self.scan.begin_accounting(frame)?;
        observe_accounting(frame, &mut self.scan, content)?;
        self.scan.end_accounting(frame.hour);
        let census_starts = self.publish_census(frame)?;
        self.scan.prune_publications(frame);
        self.settle_native(changes)?;
        self.read_daily_keys(frame, changes);
        let tags = std::mem::take(&mut self.scan.dirty);
        let mut presented = Presentations::new(&tags);
        for tag in &tags {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.history_visits += 1;
            }
            cancelled_request(cancelled)?;
            let (reply, tooltip_order, notes) =
                presented.take(&self.scan, frame.branch, tag, content, cancelled)?;
            self.open_notes
                .entry(tag.clone())
                .or_default()
                .extend(notes.facts);
            self.metadata
                .formats
                .entry(tag.clone())
                .or_default()
                .extend(notes.formats);
            for (order, (token, cost)) in tooltip_order.into_iter().enumerate() {
                self.metadata
                    .tooltips
                    .entry(series::field_key(tag, &token))
                    .or_default()[usize::from(cost)]
                .get_or_insert((frame.hour, order as u64));
            }
            let current_days: Vec<_> = self.scan.countries[tag]
                .daily
                .iter()
                .filter(|day| self.current_daily_keys.contains(&day.row.key()))
                .map(Daily::ledger)
                .collect();
            let current = current_days.iter().max_by_key(|day| day.day_hour).cloned();
            if current_days
                .iter()
                .any(|row| row.day_hour == frame.hour.div_euclid(24) * 24)
                && let Some(next) = frame
                    .hour
                    .div_euclid(24)
                    .checked_add(1)
                    .and_then(|day| day.checked_mul(24))
            {
                self.day_tags.entry(next).or_default().insert(tag.clone());
            }
            let identity = self.scan.countries[tag]
                .coverage
                .last()
                .map(|row| (row.country, row.instance, row.epoch.clone()))
                .unwrap_or_default();
            let starts = self.identities.get(tag) != Some(&identity);
            let capture = reply
                .coverage
                .last()
                .filter(|row| row.through_hour == Some(frame.hour));
            if let Some(row) = capture {
                let mut signature = row.clone();
                signature.through_hour = None;
                let changed = self.last_capture.get(tag) != Some(&signature);
                self.scalar(
                    tag,
                    "capture",
                    frame.hour,
                    Some(1),
                    starts || census_starts,
                    changed,
                )?;
                if changed {
                    let key = self.key(tag, "capture:events")?;
                    self.series
                        .event(key, frame.hour, &Event::Coverage { row: row.clone() })?;
                    self.last_capture.insert(tag.clone(), signature);
                }
            } else {
                self.scalar(tag, "capture", frame.hour, None, false, false)?;
            }

            for (field, value) in current
                .as_ref()
                .map(daily_values)
                .transpose()?
                .unwrap_or_default()
            {
                self.scalar(tag, field, frame.hour, value, false, false)?;
            }
            if current.is_none() {
                for field in DAILY_FIELDS {
                    self.scalar(tag, field, frame.hour, None, false, false)?;
                }
            }
            for day in current_days {
                let identity = (day.country, day.instance, day.epoch.clone());
                self.metadata
                    .actors
                    .entry(tag.clone())
                    .or_default()
                    .insert(identity.clone());
                let lags = self
                    .metadata
                    .daily_lags
                    .entry(series::field_key(tag, &daily_prefix(&identity)?))
                    .or_default();
                for (index, read) in [day.pp_read_hour, day.cp_read_hour].into_iter().enumerate() {
                    lags[index] =
                        lags[index].max(read.unwrap_or(day.read_hour).saturating_sub(day.day_hour));
                }
                self.metadata.largest_lag = self.metadata.largest_lag.max(
                    day.pp_read_hour
                        .into_iter()
                        .chain(day.cp_read_hour)
                        .map(|read| read.saturating_sub(day.day_hour))
                        .max()
                        .unwrap_or(0),
                );
                if self
                    .frontier
                    .is_some_and(|frontier| day.day_hour <= frontier)
                {
                    if day.read_hour == frame.hour {
                        let key = self.key(tag, "late-daily")?;
                        self.series
                            .event(key, frame.hour, &Event::LateDaily { row: day })?;
                    }
                } else {
                    self.tail
                        .entry(day.day_hour)
                        .or_default()
                        .daily
                        .insert(daily_id(tag, &day)?, day);
                }
            }
            let cover = reply.spending_coverage.iter().find(|row| {
                row.since_hour
                    .zip(row.through_hour)
                    .is_some_and(|(first, last)| first <= frame.hour && frame.hour <= last)
            });
            for (currency, known) in [
                ("Political", cover.is_some_and(|row| row.political)),
                ("Command", cover.is_some_and(|row| row.command)),
            ] {
                self.scalar(
                    tag,
                    &format!("{currency}:coverage"),
                    frame.hour,
                    known.then_some(1),
                    starts,
                    false,
                )?;
            }
            self.record_parts(&reply, frame.hour)?;
            // Coverage metadata is emitted only at a change, numeric coverage is a gap.
            if let Some(row) = cover {
                let value = (row.political, row.command, row.reason.clone());
                if self.last_cover.get(tag) != Some(&value) {
                    let key = self.key(tag, "coverage-reason")?;
                    self.series.event(key, frame.hour, row)?;
                    self.last_cover.insert(tag.clone(), value);
                }
            }
            for row in &reply.command_holds {
                if row.read_hour == frame.hour
                    || hold_boundaries.contains(tag) && row.through_hour == frame.hour
                {
                    let key = self.key(tag, "holds")?;
                    self.series
                        .event(key, frame.hour, &Event::Hold { row: row.clone() })?;
                }
            }
            for cost in reply
                .daily_costs
                .iter()
                .filter(|cost| cost.cause == "DailyGain")
            {
                let field = raw_cost_field(cost)?;
                let mut template = cost.clone();
                template.day_hour = 0;
                template.read_hour = 0;
                template.amount_raw = "0".into();
                template.pp_sequence = "0".into();
                template.covered = false;
                self.metadata
                    .costs
                    .insert(series::field_key(tag, &field), template);
                if self
                    .frontier
                    .is_some_and(|frontier| cost.day_hour <= frontier)
                {
                    if cost.read_hour == frame.hour {
                        let key = self.key(tag, "late-costs")?;
                        self.series.event(
                            key,
                            frame.hour,
                            &Event::RawCost { row: cost.clone() },
                        )?;
                    }
                } else {
                    self.tail
                        .entry(cost.day_hour)
                        .or_default()
                        .costs
                        .insert(series::field_key(tag, &field), cost.clone());
                }
            }
            let mut payments = BTreeMap::new();
            let mut projection = Payments::new(&reply);
            for spend in &reply.spending {
                let mut projected = projection.payment(spend)?;
                if projected.spend.category == "upkeep" {
                    let day = projected.spend.hour.div_euclid(24) * 24;
                    if let Some(first) = self.tail.get(&day).and_then(|open| {
                        open.payments.values().find(|old| {
                            old.spend.category == "upkeep"
                                && old.identity == projected.identity
                                && old.spend.country_tag == projected.spend.country_tag
                        })
                    }) {
                        projected.item = first.item.clone();
                        projected.category = first.category.clone();
                        projected.ideas = first.ideas.clone();
                        projected.spend.item_id = first.spend.item_id.clone();
                    }
                }

                if spend.id.starts_with("daily:")
                    && let Some(read) = reply
                        .daily
                        .iter()
                        .filter(|day| day.country == spend.country_id && day.day_hour == spend.hour)
                        .filter_map(|day| day.pp_read_hour)
                        .max()
                {
                    projected.spend.read_hour = projected.spend.read_hour.max(read);
                    for change in &mut projected.spend.changes {
                        change.read_hour = change.read_hour.max(read);
                    }
                }
                let cost = spend.id.starts_with("daily:");
                let order = projection.order.get(&spend.id).copied().unwrap_or(0);
                self.register(tag, &projected, frame.hour, order as u64, cost)?;
                payments.insert(spend.id.clone(), projected);
            }
            for row in &reply.mutations {
                let key = self.key(tag, &format!("{}:events", row.currency))?;
                self.metadata.largest_lag = self
                    .metadata
                    .largest_lag
                    .max(row.read_hour.saturating_sub(row.hour));
                let balance = reply
                    .balance_changes
                    .iter()
                    .find(|change| {
                        change.hour == row.hour
                            && change.read_hour == row.read_hour
                            && change.currency == row.currency
                            && change.realized_delta_raw == row.realized_delta_raw
                    })
                    .cloned();
                let projected = payments.remove(&mutation_id(row));
                let payment = match projected {
                    Some(projected) if is_recurring(&projected.spend.category) => {
                        let event_payment = (row.currency == "Command").then(|| projected.clone());
                        self.recurring_payment(tag, projected, None)?;
                        event_payment
                    }
                    other => other,
                };
                if let Some(projected) = &payment
                    && !is_recurring(&projected.spend.category)
                {
                    self.publish_payment(tag, row.read_hour, projected)?;
                    let payments = self.key(tag, &format!("{}:payments", row.currency))?;
                    self.series.event(
                        payments,
                        row.read_hour,
                        &Event::Payment {
                            payment: projected.clone(),
                        },
                    )?;
                }
                self.series.event(
                    key,
                    row.read_hour,
                    &Event::Mutation {
                        row: row.clone(),
                        balance,
                        payment,
                    },
                )?;
            }
            for row in &reply.governments {
                // Exact viewport joins reach the supplying read as well as the
                // native date. This event key is their publication-hour index.
                let supplying = self.key(tag, "governments:supplying")?;
                self.series.event(
                    supplying,
                    row.read_hour,
                    &Event::Government { row: row.clone() },
                )?;
                let day = row.hour.div_euclid(24) * 24;
                if self.frontier.is_some_and(|frontier| day <= frontier) {
                    let key = self.key(tag, "late-governments")?;
                    self.series.event(
                        key,
                        row.read_hour,
                        &Event::Government { row: row.clone() },
                    )?;
                } else {
                    self.metadata.government_lag = self
                        .metadata
                        .government_lag
                        .max(row.read_hour.saturating_sub(row.hour));

                    self.tail
                        .entry(day)
                        .or_default()
                        .governments
                        .push((tag.clone(), row.clone()));
                }
            }
            for row in &reply.daily_costs {
                if let Some(projected) = payments.remove(&format!(
                    "daily:{}:{}:{}:{}:{}",
                    row.country, row.instance, row.epoch, row.day_hour, row.ordinal
                )) {
                    if is_recurring(&projected.spend.category) {
                        let cost = row.clone();

                        let mut template = cost.clone();
                        template.day_hour = 0;
                        template.read_hour = 0;
                        template.amount_raw = "0".into();
                        template.pp_sequence = "0".into();
                        template.covered = false;
                        self.metadata
                            .costs
                            .insert(series::field_key(tag, &raw_cost_field(&cost)?), template);
                        self.recurring_payment(tag, projected, Some(cost))?;
                    } else {
                        let key = self.key(tag, "Political:events")?;
                        self.series.event(
                            key,
                            frame.hour,
                            &Event::Cost {
                                row: row.clone(),
                                payment: projected.clone(),
                            },
                        )?;
                        self.publish_payment(tag, frame.hour, &projected)?;
                        let key = self.key(tag, "Political:payments")?;
                        self.series.event(
                            key,
                            frame.hour,
                            &Event::Payment { payment: projected },
                        )?;
                    }
                }
            }
            self.scan.countries.get_mut(tag).unwrap().mutations.clear();
            self.scan
                .countries
                .get_mut(tag)
                .unwrap()
                .governments
                .clear();
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.folds[1] += usize::from(presented.parallel);
        }
        for tag in &tags {
            let identity = self.scan.countries[tag]
                .coverage
                .last()
                .filter(|row| row.through_hour == Some(frame.hour))
                .and_then(|row| {
                    row.country.map(|country| {
                        (
                            country,
                            row.instance.unwrap_or(0),
                            row.epoch.clone().unwrap_or_else(|| "0".into()),
                        )
                    })
                });
            let identity_state = identity
                .as_ref()
                .map(|(country, instance, epoch)| {
                    (Some(*country), Some(*instance), Some(epoch.clone()))
                })
                .unwrap_or_default();
            let starts = self.identities.get(tag).map(|old| old.0) != Some(identity_state.0)
                || self
                    .scan
                    .clock
                    .previous
                    .and_then(|hour| hour.checked_add(1))
                    != Some(frame.hour);
            self.identities.insert(tag.clone(), identity_state);
            self.scalar(
                tag,
                "parts:identity",
                frame.hour,
                identity.as_ref().map(|identity| i128::from(identity.0)),
                starts,
                false,
            )?;
            self.project_parts(tag, frame.hour, identity.clone(), starts)?;
        }
        self.check_parts(frame, changes)?;
        self.scan.retain_open(frame, &tags);
        for tag in &tags {
            let Some(pending) = self.open_notes.get_mut(tag) else {
                continue;
            };
            let unresolved: BTreeSet<_> = self
                .scan
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| &country.daily_costs)
                .map(|cost| {
                    let row = &cost.row;
                    format!(
                        "daily:{}:{}:{}:{}:{}",
                        row.country, row.instance, row.epoch, row.day_hour, row.ordinal
                    )
                })
                .collect();
            let completed: Vec<_> = pending
                .keys()
                .filter(|id| !unresolved.contains(*id))
                .cloned()
                .collect();
            let totals = self.metadata.presentation.entry(tag.clone()).or_default();
            for id in completed {
                for (reason, count) in pending.remove(&id).unwrap() {
                    count_reason(totals, reason, count)?;
                }
            }
            let totals = self
                .metadata
                .pending_presentation
                .entry(tag.clone())
                .or_default();
            totals.clear();
            for warnings in pending.values() {
                for (reason, count) in warnings {
                    count_reason(totals, reason.clone(), *count)?;
                }
            }
        }
        Ok(())
    }

    /// An hour that only advances captures ([`Scanned::advances_only`]):
    /// what the whole accounting does there, in its order. It selects no
    /// country, so it presents, projects, settles and notes none: no
    /// country's capture, coverage, holds, events, daily rows, costs or
    /// parts change, and every series key it would observe holds its value.
    /// Its accounting is [`Scanned::advance_accounting`]; then the census
    /// and accepted hour are observed, publications pruned, native days
    /// settled, the current daily keys and the captured parts kept as the
    /// hour reads them, and the closed census runs dropped.
    fn observe_advance(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.scan.advance_accounting(frame, content)?;
        self.publish_census(frame)?;
        self.scan.prune_publications(frame);
        self.settle_native(changes)?;
        self.read_daily_keys(frame, changes);
        self.check_parts(frame, changes)?;
        self.scan.retain_open(frame, &BTreeSet::new());
        Ok(())
    }

    /// Observe whether the hour's census is known and that the hour was
    /// accepted, and publish a census run that began since the last one
    /// published. Whether the hour starts an episode after a gap.
    fn publish_census(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        let census_known = frame.complete::<tables::Countries>().is_ok();
        let census_starts = self
            .scan
            .clock
            .previous
            .and_then(|last| last.checked_add(1))
            != Some(frame.hour);
        self.scalar(
            "*",
            "census",
            frame.hour,
            census_known.then_some(1),
            census_starts,
            false,
        )?;
        self.scalar("*", "accepted", frame.hour, Some(1), census_starts, false)?;
        if census_known
            && self
                .scan
                .presence
                .last()
                .is_some_and(|row| self.last_presence != Some(row.first))
        {
            let row = self.scan.presence.last().unwrap().clone();
            let key = self.key("*", "census:events")?;
            self.series
                .event(key, frame.hour, &Event::Presence { row: row.clone() })?;
            self.last_presence = Some(row.first);
        }
        Ok(census_starts)
    }

    /// Settle the open native days before the newest day the hour's power,
    /// government and daily rows publish.
    fn settle_native(&mut self, changes: &FrameChanges) -> io::Result<()> {
        let native_changed = [
            TableId::CountryPowerDaily,
            TableId::CountryPowerChanges,
            TableId::GovernmentChanges,
        ]
        .iter()
        .any(|table| changes.rows_changed[table.index()] || changes.table_read[table.index()]);
        if native_changed {
            let native = &self.scan.inputs;
            // The power and government rows' newest native days are kept as
            // their rows change.
            let newest = native
                .daily()
                .iter()
                .take(if native.daily_changed {
                    native.daily().len()
                } else {
                    0
                })
                .map(|row| row.day_hour)
                .chain(native.newest_day(0).filter(|_| native.power_changed))
                .chain(native.newest_day(1).filter(|_| native.governments_changed))
                .max();
            #[cfg(test)]
            {
                let visits = if native.daily_changed {
                    native.daily().len()
                } else {
                    0
                };
                self.scan.publication_visits += visits;
            }
            if let Some(newest) = newest {
                self.settle_before(newest)?;
            }
        }
        Ok(())
    }

    /// The keys of the daily rows the hour publishes, read again only from a
    /// daily table allocation they were not read from.
    fn read_daily_keys(&mut self, frame: &DecodedFrame, changes: &FrameChanges) {
        let index = TableId::CountryPowerDaily.index();
        if !(changes.rows_changed[index] || changes.table_read[index]) {
            return;
        }
        let view = &frame.tables[index];
        if self.daily_view.as_ref().is_some_and(|read| read.is(view)) {
            return;
        }
        self.current_daily_keys = frame
            .complete::<tables::CountryPowerDaily>()
            .unwrap_or_default()
            .iter()
            .map(RowCodec::key)
            .collect();
        self.daily_view = Some(view.identity());
    }

    /// Keep the open parts of the captured countries only, when the hour
    /// reads its captures.
    fn check_parts(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        if (changes.rows_changed[TableId::PoliticsCaptureCoverage.index()]
            || changes.table_read[TableId::PoliticsCaptureCoverage.index()])
            && let Ok(rows) = frame.complete::<tables::PoliticsCaptureCoverage>()
        {
            // Changed captures describe the hour after the last one observed.
            let narrow = self.parts_checked
                && self
                    .scan
                    .clock
                    .previous
                    .and_then(|last| last.checked_add(1))
                    == Some(frame.hour);
            #[cfg(test)]
            let narrow = narrow && !self.scan.whole_tables;
            match changes
                .rows(TableId::PoliticsCaptureCoverage)
                .filter(|_| narrow)
            {
                // Every other part named a capture of the last read, so only
                // the deleted captures and the parts added since can be gone.
                Some((_, deleted)) => {
                    for &key in deleted {
                        self.parts.remove(&serde_json::to_string(&(key as u32))?);
                    }
                    for identity in std::mem::take(&mut self.new_parts) {
                        let captured = identity.parse::<u32>().is_ok_and(|country| {
                            self.scan.inputs.witnesses.contains_key(&country)
                                && serde_json::to_string(&country).is_ok_and(|key| key == identity)
                        });
                        if !captured {
                            self.parts.remove(&identity);
                        }
                    }
                }
                None => {
                    let current: BTreeSet<_> = rows
                        .iter()
                        .map(|row| serde_json::to_string(&row.country))
                        .collect::<Result<_, _>>()?;
                    self.parts.retain(|identity, _| current.contains(identity));
                }
            }
            self.new_parts.clear();
            self.parts_checked = true;
        }
        Ok(())
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.materialize_clock()?;
        if let Some(frame) = self.scan.previous.clone() {
            self.scan.compact_open(&frame);
        }
        self.metadata.warnings = self
            .scan
            .warnings
            .iter()
            .map(|(scope, reasons)| (scope.clone(), reasons.clone()))
            .collect();
        self.series
            .flush(writer, &format!("politics-ledger-{branch}"), 5, cancelled)?;
        self.recurring.flush(
            writer,
            &format!("politics-ledger-recurring-{branch}"),
            5,
            cancelled,
        )?;
        let mut tails = BTreeMap::<String, BTreeMap<i32, OpenDay>>::new();
        for (&day, open) in &self.tail {
            for (tag, row) in &open.governments {
                tails
                    .entry(tag.clone())
                    .or_default()
                    .entry(day)
                    .or_default()
                    .governments
                    .push((tag.clone(), row.clone()));
            }
            for (id, payment) in &open.payments {
                tails
                    .entry(payment.spend.country_tag.clone())
                    .or_default()
                    .entry(day)
                    .or_default()
                    .payments
                    .insert(id.clone(), payment.clone());
            }
            for (id, row) in &open.daily {
                let (tag, _): (String, String) = serde_json::from_str(id)?;
                tails
                    .entry(tag)
                    .or_default()
                    .entry(day)
                    .or_default()
                    .daily
                    .insert(id.clone(), row.clone());
            }
        }
        for (&day, open) in &self.tail {
            for (id, row) in &open.costs {
                let (tag, _): (String, String) = serde_json::from_str(id)?;
                tails
                    .entry(tag)
                    .or_default()
                    .entry(day)
                    .or_default()
                    .costs
                    .insert(id.clone(), row.clone());
            }
        }
        for tag in self.scan.countries.keys() {
            let next = u64::try_from(self.metadata.tail_ranges.len())
                .map_err(|_| dashboard_core::invalid("Politics tail range count overflow"))?;
            self.metadata.tail_ranges.entry(tag.clone()).or_insert(next);
        }
        // Tail file ordinals belong to their own section, not the scalar key
        // space. Write every country's range, even empty, in append order.
        let mut ranges: Vec<_> = self.metadata.tail_ranges.iter().collect();
        ranges.sort_unstable_by_key(|(_, range)| **range);
        for (tag, &range) in ranges {
            let tail = tails.remove(tag).unwrap_or_default();
            series::write_reply(
                writer,
                &format!("politics-ledger-tail-{branch}"),
                range,
                &tail,
            )?;
        }
        series::write_catalogue(
            writer,
            "politics-ledger",
            branch,
            &self.keys.flat(),
            &self.metadata,
        )?;
        Ok(())
    }
    pub fn saved_bytes(&mut self) -> io::Result<Vec<u8>> {
        self.scan.materialize_clock()?;
        dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 1)
    }
    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::read::Decoder::new(
            reader,
        )?)?)
    }
}

/// The tables a block's join reads ([`BlockExtract`]): the census and
/// capture coverage, the native power, government, daily and cost tables,
/// the command power allocations, the characters, portraits and dictionary
/// whose reads change it, and the campaign identity every block read keeps.
pub const BLOCK_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::PoliticsCaptureCoverage,
    TableId::CountryPowerChanges,
    TableId::GovernmentChanges,
    TableId::CountryPowerDaily,
    TableId::CountryPowerDailyCosts,
    TableId::CountryCommandPower,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// What a pre-pass worker read of one block for the ledger's fold: the join
/// the block's first hour leaves, and the later hours the join certifies.
///
/// The worker joins the block read alone from its first hour, as the
/// ledger's own join does ([`BLOCK_TABLES`], with the change facts of the
/// block alone, which are the observer's), and none of the ledger's
/// accounting. At an hour it certifies, its join from the hour's changed
/// rows extends at least one capture to the hour, every changed capture
/// only extends (same identity, first hour and streams, no power row dated
/// after its old last hour), and nothing else joins a country: no census,
/// link, capture, power, government, allocation, completeness, daily gain
/// or dictionary change. With none of the rows not yet recorded as events,
/// which are the observer's own, observed again. A join's state after an
/// hour follows from its state before and the hour's tables and changes
/// alone, and so is the ledger's at every later hour of the block once it
/// is after the first.
pub struct BlockFacts {
    /// The block's first hour and the join it leaves.
    first: Option<(i32, Box<inputs::Inputs>)>,
    /// Runs of consecutive certified hours extending the same captures: the
    /// first and last hour and the captures, each with its first hour, in
    /// country order.
    spans: Vec<(i32, i32, Arc<[(u32, i32)]>)>,
}

/// A fresh join of one block on a pre-pass worker ([`BlockFacts`]).
pub struct BlockExtract {
    inputs: inputs::Inputs,
    previous: Option<i32>,
    facts: BlockFacts,
}

/// The ledger's block extract.
pub fn block_extract(_: replay::format::FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        inputs: inputs::Inputs::default(),
        previous: None,
        facts: BlockFacts {
            first: None,
            spans: Vec::new(),
        },
    })
}

/// The rows a worker's join treats as not yet recorded: none, so an hour it
/// certifies joins no country by itself; the fold checks the observer's.
static NO_PENDING: [BTreeSet<u128>; 2] = [BTreeSet::new(), BTreeSet::new()];

impl Extract for BlockExtract {
    /// What the block's facts hold: the join its first hour leaves and the
    /// captures of each certified span.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        // The join (its box and all it keeps), and each span's captures,
        // each shared allocation once.
        held.heap(&self.facts.first).heap(&self.facts.spans).total()
    }

    fn observe(
        &mut self,
        _: &replay::container::Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        // As the observer joins: from the changed rows when the hour
        // continues the last, otherwise from the whole tables.
        let continuous = self.previous.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        self.inputs
            .refresh_changes(frame, changes, continuous.then_some(&NO_PENDING));
        self.previous = Some(frame.hour);
        let inputs = &self.inputs;
        if self.facts.first.is_none() {
            self.facts.first = Some((frame.hour, Box::new(inputs.clone())));
        } else if inputs.narrow && inputs.joins_nothing() && !inputs.advanced.is_empty() {
            match self.facts.spans.last_mut() {
                Some((_, last, advanced))
                    if last.checked_add(1) == Some(frame.hour)
                        && **advanced == *inputs.advanced =>
                {
                    *last = frame.hour;
                }
                _ => self.facts.spans.push((
                    frame.hour,
                    frame.hour,
                    Arc::from(inputs.advanced.as_slice()),
                )),
            }
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn std::any::Any + Send>> {
        Ok(Box::new(self.facts))
    }
}

/// A block's [`BlockFacts`] as its fold goes through its hours.
pub struct BlockFold {
    facts: BlockFacts,
    /// None before the block's first hour; then whether the join it left is
    /// the worker's.
    joined: Option<bool>,
    /// The first span that may hold a later hour.
    span: usize,
}

impl BlockFold {
    pub fn new(facts: BlockFacts) -> Self {
        Self {
            facts,
            joined: None,
            span: 0,
        }
    }

    /// The captures `hour` extends when the worker certified it. Hours are
    /// asked in order.
    fn certified(&mut self, hour: i32) -> Option<Arc<[(u32, i32)]>> {
        let spans = &self.facts.spans;
        while spans
            .get(self.span)
            .is_some_and(|&(_, last, _)| last < hour)
        {
            self.span += 1;
        }
        spans
            .get(self.span)
            .filter(|&&(first, _, _)| first <= hour)
            .map(|(_, _, advanced)| Arc::clone(advanced))
    }
}

/// One country's presentation at an hour: its raw reply, the tooltip order
/// and the presentation notes.
type Presented = (PoliticsLedgerReply, Vec<(String, bool)>, PresentationNotes);

fn present_one(
    scan: &Scanned,
    branch: u64,
    tag: &str,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
) -> io::Result<Presented> {
    let mut tooltip_order = Vec::new();
    let mut notes = PresentationNotes::default();
    let reply = present_fields(
        scan,
        branch,
        tag,
        content,
        cancelled,
        LedgerView::Raw,
        true,
        false,
        Some(&mut tooltip_order),
        Some(&mut notes),
    )?;
    Ok((reply, tooltip_order, notes))
}

/// The countries an hour presents, taken in order. A country's presentation
/// reads its own rows, holds and coverage and what every country shares (the
/// branch's warnings, names, census runs, modifier definitions and read
/// extent), none of which taking an earlier country changes: that clears
/// only the earlier country's own mutations and governments. So a window of
/// countries is presented at once on several threads when enough remain,
/// and each is what presenting it in turn makes. The content's power
/// diagnostics, which a presentation reports once any item demanded the
/// power content, must already be reachable: no presentation then changes
/// what another reports.
struct Presentations<'t> {
    tags: Vec<&'t String>,
    next: usize,
    ready: std::collections::VecDeque<io::Result<Presented>>,
    #[cfg(any(test, feature = "test-support"))]
    parallel: bool,
}

impl<'t> Presentations<'t> {
    fn new(tags: &'t BTreeSet<String>) -> Self {
        Self {
            tags: tags.iter().collect(),
            next: 0,
            ready: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            parallel: false,
        }
    }

    /// `tag`'s presentation: the next country's.
    fn take(
        &mut self,
        scan: &Scanned,
        branch: u64,
        tag: &str,
        content: Option<&PoliticsContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<Presented> {
        debug_assert_eq!(self.tags.get(self.next).map(|tag| tag.as_str()), Some(tag));
        if self.ready.is_empty() {
            let rest = &self.tags[self.next..];
            let (least, threads) = presentation_threads();
            if threads > 1
                && rest.len() >= least
                && content.is_none_or(PoliticsContent::power_requested)
            {
                let window = &rest[..rest.len().min(threads.saturating_mul(PRESENTED_PER_THREAD))];
                self.ready.extend(present_window(
                    scan, branch, window, content, cancelled, threads,
                ));
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.parallel = true;
                }
            }
        }
        self.next += 1;
        match self.ready.pop_front() {
            Some(presented) => presented,
            None => present_one(scan, branch, tag, content, cancelled),
        }
    }
}

/// Countries each thread presents in one window.
const PRESENTED_PER_THREAD: usize = 32;

/// The fewest countries left to present on several threads, and the
/// threads: the machine's processors.
fn presentation_threads() -> (usize, usize) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(test) = TEST_PRESENTATION.with(std::cell::Cell::get) {
        return test;
    }
    static THREADS: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    });
    (32, *THREADS)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_PRESENTATION: std::cell::Cell<Option<(usize, usize)>> =
        const { std::cell::Cell::new(None) };
}

/// Run `run` presenting on `threads` threads whenever at least `least`
/// countries are left to present at an hour.
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_presentation<T>(least: usize, threads: usize, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<(usize, usize)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PRESENTATION.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_PRESENTATION.with(|value| value.replace(Some((least, threads)))));
    run()
}

/// `tags` presented on up to `threads` threads, in order. A thread that
/// cannot start leaves its countries to this one.
fn present_window(
    scan: &Scanned,
    branch: u64,
    tags: &[&String],
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
    threads: usize,
) -> Vec<io::Result<Presented>> {
    let size = tags.len().div_ceil(threads.max(1)).max(1);
    std::thread::scope(|scope| {
        let parts: Vec<_> = tags
            .chunks(size)
            .map(|part| {
                let priority = JobPriority::current();
                let started = std::thread::Builder::new()
                    .name("politics ledger presentation".into())
                    .spawn_scoped(scope, move || {
                        priority.enter(Level::Normal);
                        part.iter()
                            .map(|tag| present_one(scan, branch, tag, content, cancelled))
                            .collect::<Vec<_>>()
                    });
                (part, started)
            })
            .collect();
        let mut presented = Vec::with_capacity(tags.len());
        for (part, started) in parts {
            match started {
                Ok(handle) => match handle.join() {
                    Ok(rows) => presented.extend(rows),
                    Err(payload) => std::panic::resume_unwind(payload),
                },
                Err(_) => presented.extend(
                    part.iter()
                        .map(|tag| present_one(scan, branch, tag, content, cancelled)),
                ),
            }
        }
        presented
    })
}

const DAILY_FIELDS: &[&str] = &[
    "Political:gross",
    "Political:cost",
    "Political:income",
    "Political:realized",
    "Political:lag",
    "Political:sequence",
    "Command:income",
    "Command:realized",
    "Command:lag",
    "Command:sequence",
    "country",
    "instance",
    "epoch",
    "read",
];
fn daily_values(day: &LedgerDaily) -> io::Result<Vec<(&'static str, Option<i128>)>> {
    Ok(vec![
        ("Political:gross", amount(day.pp_gross_raw.as_deref())?),
        ("Political:cost", amount(day.pp_cost_raw.as_deref())?),
        ("Political:income", amount(day.pp_income_raw.as_deref())?),
        (
            "Political:realized",
            amount(day.pp_realized_raw.as_deref())?,
        ),
        (
            "Political:lag",
            day.pp_read_hour
                .map(|hour| i128::from(hour) - i128::from(day.day_hour)),
        ),
        ("Political:sequence", amount(day.pp_sequence.as_deref())?),
        ("Command:income", amount(day.cp_income_raw.as_deref())?),
        ("Command:realized", amount(day.cp_realized_raw.as_deref())?),
        (
            "Command:lag",
            day.cp_read_hour
                .map(|hour| i128::from(hour) - i128::from(day.day_hour)),
        ),
        ("Command:sequence", amount(day.cp_sequence.as_deref())?),
        ("country", Some(i128::from(day.country))),
        ("instance", Some(i128::from(day.instance))),
        ("epoch", amount(Some(&day.epoch))?),
        ("read", Some(i128::from(day.read_hour))),
    ])
}

mod query;
pub use query::query;
#[cfg(test)]
mod tests;

mod scan_state {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        scan: &Scanned,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        Continuation::from_scan(scan)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Scanned, D::Error> {
        Continuation::deserialize(deserializer)?
            .into_scan()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
struct OpenCountry {
    daily: Vec<(u128, i32, Option<i32>, Option<i32>)>,
    costs: Vec<(u128, i32, bool, EventFacts)>,
    coverage: Vec<LedgerCoverage>,
    spending_coverage: Vec<LedgerSpendCoverage>,
}

/// Raw operands exist only for the current tuple and unresolved applications.
/// Finished observations are published as series/events before this is saved.
#[derive(Serialize, Deserialize)]
#[allow(clippy::type_complexity)]
struct Continuation {
    clock: clock::Clock,
    rows: Vec<u8>,
    countries: BTreeMap<String, OpenCountry>,
    identities: Vec<((u32, u32, u64), BTreeMap<i32, String>)>,
    events: BTreeSet<(u32, u32, u64, u64)>,
    published_events: [BTreeSet<(u32, u32, u64, u64)>; 2],
    costs: BTreeSet<(u32, u32, u64, i32, u32)>,
    naming: Vec<((u32, i32), Arc<NamedCountry>)>,
    modifiers: Option<Vec<(String, u128)>>,
    warnings: Vec<(Option<String>, BTreeMap<String, u64>)>,
    presence: Vec<PresenceRun>,
    known_tags: BTreeMap<u32, String>,
    refused: BTreeMap<u32, (String, u64)>,
    holds: holds::Holds,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
}

impl Continuation {
    fn from_scan(scan: &Scanned) -> io::Result<Self> {
        let mut days: Vec<_> = scan
            .countries
            .values()
            .flat_map(|country| country.daily.iter().map(|day| day.row.clone()))
            .collect();
        days.sort_unstable_by_key(RowCodec::key);
        let mut costs: Vec<_> = scan
            .countries
            .values()
            .flat_map(|country| country.daily_costs.iter().map(|cost| cost.row.clone()))
            .collect();
        costs.sort_unstable_by_key(RowCodec::key);
        let mut definitions: Vec<_> = scan
            .modifier_definitions
            .iter()
            .flat_map(|rows| rows.values().cloned())
            .collect();
        definitions.sort_unstable_by_key(RowCodec::key);
        definitions.dedup_by_key(|row| row.key());
        Ok(Self {
            clock: scan.clock.clone(),
            rows: checkpoint_frame(
                scan.last_hour.unwrap_or(0),
                vec![
                    TableRows::CountryPowerDaily(days),
                    TableRows::CountryPowerDailyCosts(costs),
                    TableRows::ModifierDefinitions(definitions),
                ],
            )?,
            countries: scan
                .countries
                .iter()
                .map(|(tag, country)| {
                    (
                        tag.clone(),
                        OpenCountry {
                            daily: country
                                .daily
                                .iter()
                                .map(|day| {
                                    (
                                        day.row.key(),
                                        day.read_hour,
                                        day.pp_read_hour,
                                        day.cp_read_hour,
                                    )
                                })
                                .collect(),
                            costs: country
                                .daily_costs
                                .iter()
                                .map(|cost| {
                                    (
                                        cost.row.key(),
                                        cost.read_hour,
                                        cost.covered,
                                        cost.facts.clone(),
                                    )
                                })
                                .collect(),
                            coverage: country.coverage.clone(),
                            spending_coverage: country.spending_coverage.clone(),
                        },
                    )
                })
                .collect(),
            identities: scan
                .identities
                .iter()
                .map(|(key, value)| (*key, value.clone()))
                .collect(),
            events: scan.events.clone(),
            published_events: scan.published_events.clone(),
            costs: scan.costs.clone(),
            naming: scan
                .naming
                .iter()
                .map(|(key, value)| (*key, Arc::clone(value)))
                .collect(),
            modifiers: scan.modifier_definitions.as_ref().map(|rows| {
                rows.iter()
                    .map(|(name, row)| (name.clone(), row.key()))
                    .collect()
            }),
            warnings: scan
                .warnings
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            presence: scan.presence.clone(),
            known_tags: scan.known_tags.clone(),
            refused: scan.refused.clone(),
            holds: scan.holds.clone(),
            first_hour: scan.first_hour,
            last_hour: scan.last_hour,
        })
    }

    fn into_scan(self) -> io::Result<Scanned> {
        let frame = read_checkpoint(&self.rows)?;
        let days: BTreeMap<_, _> = frame
            .complete::<tables::CountryPowerDaily>()?
            .iter()
            .map(|row| (row.key(), row))
            .collect();
        let costs: BTreeMap<_, _> = frame
            .complete::<tables::CountryPowerDailyCosts>()?
            .iter()
            .map(|row| (row.key(), row))
            .collect();
        let definitions: BTreeMap<_, _> = frame
            .complete::<tables::ModifierDefinitions>()?
            .iter()
            .map(|row| (row.key(), row))
            .collect();
        let mut scan = Scanned {
            clock: self.clock,
            identities: self.identities.into_iter().collect(),
            events: self.events,
            published_events: self.published_events,
            costs: self.costs,
            naming: self.naming.into_iter().collect(),
            warnings: self.warnings.into_iter().collect(),
            presence: self.presence,
            known_tags: self.known_tags,
            refused: self.refused,
            holds: self.holds,
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            ..Default::default()
        };
        for (tag, country) in self.countries {
            let mut open = Country {
                coverage: country.coverage,
                spending_coverage: country.spending_coverage,
                ..Default::default()
            };
            for (key, read_hour, pp_read_hour, cp_read_hour) in country.daily {
                let row = (*days
                    .get(&key)
                    .ok_or_else(|| dashboard_core::invalid("Missing open politics day"))?)
                .clone();
                scan.daily.insert(
                    (row.country, row.instance, row.epoch, row.day_hour),
                    open.daily.len(),
                );
                open.daily.push(Daily {
                    row,
                    read_hour,
                    pp_read_hour,
                    cp_read_hour,
                });
            }
            for (key, read_hour, covered, facts) in country.costs {
                let row = (*costs
                    .get(&key)
                    .ok_or_else(|| dashboard_core::invalid("Missing unresolved politics cost"))?)
                .clone();
                open.daily_costs.push(DailyCost {
                    row,
                    read_hour,
                    covered,
                    facts,
                });
            }
            scan.countries.insert(tag, open);
        }
        scan.modifier_definitions = self
            .modifiers
            .map(|rows| {
                rows.into_iter()
                    .map(|(name, key)| {
                        let row = (*definitions.get(&key).ok_or_else(|| {
                            dashboard_core::invalid("Missing open politics modifier")
                        })?)
                        .clone();
                        Ok((name, row))
                    })
                    .collect::<io::Result<_>>()
            })
            .transpose()?;
        Ok(scan)
    }
}

pub(super) fn checkpoint_frame(hour: i32, rows: Vec<TableRows>) -> io::Result<Vec<u8>> {
    let mut frame = DecodedFrame {
        branch: 0,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| {
            replay::frame::TableView::Unavailable {
                reason: "Unused ledger continuation input".into(),
            }
            .into()
        }),
    };
    for rows in rows {
        let table = rows.table();
        frame.tables[table.index()] = replay::frame::TableView::Complete(rows).into();
    }
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn read_checkpoint(bytes: &[u8]) -> io::Result<DecodedFrame> {
    let mut decoder = replay::codec::Decoder::new(bytes);
    let frame = DecodedFrame::decode_checkpoint(&mut decoder)?;
    decoder.finish()?;
    Ok(frame)
}
