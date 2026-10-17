//! A shared projection of payments. Signed PP operands are applied then
//! clamped; CP subtracts its operand. Upkeep commits on each paid activation
//! anniversary. Reconciliation, grants and refunds are never payments.
use super::*;
use dashboard_base::products::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
use dashboard_base::products::politics_content::{PoliticsContent, politics_content};
use dashboard_base::products::{
    idea_format::{self, LocalisedText, ModifierFormat},
    idea_history::{self, IdeaInfo},
    validate_tag,
};
use dashboard_core::transport::json_len;
#[cfg(test)]
use dashboard_text::catalogue::Catalogue;
use dashboard_text::catalogue::{View, plain_text};
use dashboard_text::identity::{Look, NamingCountry, character_script_token};
use installed_content::country_names::CountryNames;
use std::sync::Arc;
mod facts;
use facts::{EventFacts, event_facts};
mod presentation;
use presentation::{count_reason, fill_gap, intern_tooltip, invalidate_read};
mod holds;
pub mod prepared;
pub use holds::{LedgerCommandHold, LedgerCommandHolder};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerIdeaTooltips {
    pub ideas: Vec<IdeaInfo>,
    pub artwork: Vec<dashboard_core::dto::Artwork>,
    pub modifiers: Vec<ModifierFormat>,
    pub texts: Vec<LocalisedText>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendChange {
    /// Native date of this contribution, separate from its supplying read.
    /// Older consumers may omit it; a filtered aggregate then stays unknown.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub hour: Option<i32>,
    pub read_hour: i32,
    pub charge_raw: Option<String>,
    pub realized_delta_raw: Option<String>,
    pub occurrences: u32,
    pub covered: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpend {
    /// Recorded country link identity, also CountryRow.id.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub country_id: u32,
    pub id: String,
    pub sequence: String,
    pub ordinal: u32,
    pub hour: i32,
    pub read_hour: i32,
    pub country_tag: String,
    pub country_name: Option<String>,
    pub currency: String,
    pub category: String,
    pub item_id: String,
    pub charge_raw: Option<String>,
    pub realized_delta_raw: Option<String>,
    pub covered: bool,
    pub occurrences: u32,
    /// Upkeep has one row per country/day; each contribution remains dated
    /// by its own read, so Live never reveals the rest of a day's payments.
    pub changes: Vec<LedgerSpendChange>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendItem {
    pub id: String,
    /// Specific-item identity includes the recorded target and template.
    pub group_id: String,
    pub name: Option<String>,
    /// Plain display text, using only the recorded cause/item/target.
    pub action: String,
    pub icon: Option<String>,
    pub tooltip_token: Option<String>,
    pub character: Option<u32>,
    pub target_tag: Option<String>,
    pub target_name: Option<String>,
    pub unknown: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendCategory {
    pub id: String,
    pub name: String,
    pub identified: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendCoverage {
    pub read_hour: i32,
    pub since_hour: Option<i32>,
    pub through_hour: Option<i32>,
    pub political: bool,
    pub command: bool,
    pub reason: Option<String>,
}

#[derive(Default)]
pub(super) struct Scanned {
    clock: clock::Clock,
    dirty: BTreeSet<String>,
    inputs: Arc<inputs::Inputs>,
    countries: BTreeMap<String, Country>,
    identities: BTreeMap<(u32, u32, u64), BTreeMap<i32, String>>,
    events: BTreeSet<(u32, u32, u64, u64)>,
    published_events: [BTreeSet<(u32, u32, u64, u64)>; 2],
    daily: BTreeMap<(u32, u32, u64, i32), usize>,
    costs: BTreeSet<(u32, u32, u64, i32, u32)>,
    /// A country's equal names share one allocation across its event hours.
    naming: BTreeMap<(u32, i32), Arc<NamedCountry>>,
    shared: facts::Shared,
    modifier_definitions: Option<BTreeMap<String, ModifierDefinitionRow>>,
    modifier_views: Option<(replay::frame::ViewIdentity, replay::frame::ViewIdentity)>,
    /// The previous read: whole, or only its naming tables
    /// ([`naming_frame`]) when `replay` can decode it again.
    previous: Option<DecodedFrame>,
    previous_whole: bool,
    /// The recording the reads come from. A row dated at the previous read
    /// is described from that read decoded again, so no whole read is held
    /// between hours: holding one would make the decoder copy every table
    /// the next hour changes instead of changing it in place.
    replay: Option<Arc<replay::container::Replay>>,
    /// Previous reads decoded again.
    #[cfg(test)]
    reads_again: usize,
    /// Event keys and rows visited to publish and settle this observer's reads.
    #[cfg(test)]
    publication_visits: usize,
    /// Recorded cost keys visited by publication pruning.
    #[cfg(test)]
    cost_prune_visits: usize,
    #[cfg(any(test, feature = "test-support"))]
    dated: Option<BTreeSet<i32>>,
    warnings: BTreeMap<Option<String>, BTreeMap<String, u64>>,
    presence: Vec<PresenceRun>,
    known_tags: BTreeMap<u32, String>,
    refused: BTreeMap<u32, (String, u64)>,
    holds: holds::Holds,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
    /// Power and cost row keys not recorded as events, once known.
    pending: [BTreeSet<u128>; 2],
    pending_known: bool,
    /// Whether the recorded cost keys were pruned to the current cost rows'
    /// keys since those keys last changed: costs recorded since are current
    /// rows', so pruning again removes none.
    costs_pruned: bool,
    /// Countries whose capture and holds an accepted read observed.
    #[cfg(test)]
    country_visits: usize,
    /// Join every hour from the whole tables, never from changed rows.
    #[cfg(test)]
    whole_tables: bool,
    /// Hours joined from their changed rows alone.
    #[cfg(test)]
    row_hours: usize,
}
#[cfg(any(test, feature = "test-support"))]
pub mod oracle;
#[cfg(any(test, feature = "test-support"))]
pub(super) use oracle::load;
mod clock;
mod inputs;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NamedCountry {
    tag: String,
    name: Option<String>,
    naming: NamingCountry,
    ruling_group: Result<Option<String>, String>,
}
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct ItemText {
    name: Option<String>,
    sprite: Option<String>,
    description: Option<String>,
    unknown: Option<String>,
}
/// A country's recorded rows. The cached scan serves every view and tag, so
/// rows keep their native values and are formatted only where used.
#[derive(Default)]
struct Country {
    mutations: Vec<Mutation>,
    governments: Vec<LedgerGovernment>,
    daily: Vec<Daily>,
    daily_costs: Vec<DailyCost>,
    coverage: Vec<LedgerCoverage>,
    spending_coverage: Vec<LedgerSpendCoverage>,
}
struct Mutation {
    row: CountryPowerChangeRow,
    read_hour: i32,
    covered: bool,
    facts: EventFacts,
}
#[derive(Clone, PartialEq, Eq)]
struct Daily {
    row: CountryPowerDailyRow,
    read_hour: i32,
    pp_read_hour: Option<i32>,
    cp_read_hour: Option<i32>,
}
struct DailyCost {
    row: CountryPowerDailyCostRow,
    read_hour: i32,
    covered: bool,
    facts: EventFacts,
}
impl Mutation {
    fn ledger(&self) -> LedgerMutation {
        let row = &self.row;
        LedgerMutation {
            country: row.country,
            instance: row.instance,
            sequence: row.sequence.to_string(),
            epoch: row.epoch.to_string(),
            hour: row.hour,
            read_hour: self.read_hour,
            currency: format!("{:?}", row.currency),
            cause: format!("{:?}", row.cause),
            operation: format!("{:?}", row.operation),
            site: format!("{:?}", row.site),
            amount_raw: row.requested_delta_raw.map(|raw| raw.to_string()),
            realized_delta_raw: row.realized_delta_raw().to_string(),
            before_raw: row.balance_before_raw.to_string(),
            after_raw: row.balance_after_raw.to_string(),
            item: (&row.identity).into(),
            covered: self.covered,
        }
    }
}
impl Daily {
    fn ledger(&self) -> LedgerDaily {
        let row = &self.row;
        LedgerDaily {
            country: row.country,
            instance: row.instance,
            epoch: row.epoch.to_string(),
            day_hour: row.day_hour,
            read_hour: self.read_hour,
            pp_read_hour: self.pp_read_hour,
            cp_read_hour: self.cp_read_hour,
            pp_gross_raw: row.pp_gross_gain_raw.map(|raw| raw.to_string()),
            pp_cost_raw: row.pp_cost_raw.map(|raw| raw.to_string()),
            pp_income_raw: row.pp_net_gain_raw.map(|raw| raw.to_string()),
            pp_realized_raw: row.pp_realized_delta_raw.map(|raw| raw.to_string()),
            pp_sequence: row.pp_sequence.map(|raw| raw.to_string()),
            cp_income_raw: row.cp_gain_raw.map(|raw| raw.to_string()),
            cp_realized_raw: row.cp_realized_delta_raw.map(|raw| raw.to_string()),
            cp_sequence: row.cp_sequence.map(|raw| raw.to_string()),
        }
    }
}
impl DailyCost {
    fn ledger(&self) -> LedgerDailyCost {
        let row = &self.row;
        LedgerDailyCost {
            country: row.country,
            instance: row.instance,
            epoch: row.epoch.to_string(),
            day_hour: row.day_hour,
            pp_sequence: row.pp_sequence.to_string(),
            ordinal: row.ordinal,
            read_hour: self.read_hour,
            covered: self.covered,
            component: format!("{:?}", row.component),
            amount_raw: row.cost_raw.to_string(),
            cause: format!("{:?}", row.cause),
            item: (&row.identity).into(),
        }
    }
}
fn undated_reason(reason: &str) -> String {
    let mut words = reason.split_whitespace().peekable();
    let mut result = Vec::new();
    while let Some(word) = words.next() {
        result.push(word);
        if matches!(word, "hour" | "frame")
            && words.peek().is_some_and(|next| {
                next.trim_end_matches([':', ',', ';', '.'])
                    .parse::<i32>()
                    .is_ok()
            })
        {
            words.next();
        }
    }
    result.join(" ")
}
fn combine_reason(target: &mut Option<String>, reason: Option<String>) {
    if let Some(reason) = reason {
        let reason = undated_reason(&reason);
        *target = Some(match target.take() {
            Some(previous) if previous != reason => format!("{previous}; {reason}"),
            Some(previous) => previous,
            None => reason,
        });
    }
}
impl Scanned {
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
    ) -> io::Result<()> {
        observe_all(frame, self, content)
    }

    pub(super) fn resume(&mut self, frame: &DecodedFrame) {
        self.retain_previous(frame);
        self.pending_known = false;
        self.costs_pruned = false;
        self.clock.stable = false;
        self.clock.holders = self.holds.known_tags().cloned().collect();
        // A census lost on the next read is a holder transition, as without
        // the resume; absent holders must not carry their coverage across it.
        self.clock.census_known = frame.complete::<tables::Countries>().is_ok();
        self.inputs = Default::default();
        // Rebuild the supplying census so the next read can distinguish a
        // real holder removal from a replay/resume recheck of an old tag.
        Arc::make_mut(&mut self.inputs).refresh(frame, None);
        self.clock.active = self.inputs.tag_slots.keys().cloned().collect();
    }

    /// Keep the read just observed for the next read: whole when no
    /// recording can decode it again, otherwise only its naming tables.
    fn retain_previous(&mut self, frame: &DecodedFrame) {
        let whole = self.replay.is_none();
        // The oracle holds whole only a read the next read dates a row at.
        #[cfg(any(test, feature = "test-support"))]
        let whole = whole
            && self
                .dated
                .as_ref()
                .is_none_or(|hours| hours.contains(&frame.hour));
        self.previous = Some(if whole {
            frame.clone()
        } else {
            naming_frame(frame)
        });
        self.previous_whole = whole;
    }

    fn retain_open(&mut self, frame: &DecodedFrame, selected: &BTreeSet<String>) {
        let current = frame
            .complete::<tables::CountryPowerDaily>()
            .ok()
            .map(|_| &self.inputs.daily_keys);
        let current_costs = frame
            .complete::<tables::CountryPowerDailyCosts>()
            .ok()
            .map(|_| &self.inputs.cost_keys);
        for tag in selected {
            let Some(country) = self.countries.get_mut(tag) else {
                continue;
            };
            for day in &country.daily {
                self.daily.remove(&(
                    day.row.country,
                    day.row.instance,
                    day.row.epoch,
                    day.row.day_hour,
                ));
            }
            country.mutations.clear();
            country.governments.clear();
            if let Some(current) = &current {
                country.daily.retain(|day| {
                    current.contains(&(
                        day.row.country,
                        day.row.instance,
                        day.row.epoch,
                        day.row.day_hour,
                    ))
                });
            }
            let applications: BTreeSet<_> = country
                .daily
                .iter()
                .filter_map(|day| {
                    day.row.pp_sequence.map(|sequence| {
                        (
                            day.row.country,
                            day.row.instance,
                            day.row.epoch,
                            day.row.day_hour,
                            sequence,
                        )
                    })
                })
                .collect();
            country.daily_costs.retain(|cost| {
                let row = &cost.row;
                !applications.contains(&(
                    row.country,
                    row.instance,
                    row.epoch,
                    row.day_hour,
                    row.pp_sequence,
                )) && current_costs.as_ref().is_none_or(|current| {
                    current.contains(&(
                        row.country,
                        row.instance,
                        row.epoch,
                        row.day_hour,
                        row.ordinal,
                    ))
                })
            });
            for (index, day) in country.daily.iter().enumerate() {
                let row = &day.row;
                self.daily
                    .insert((row.country, row.instance, row.epoch, row.day_hour), index);
            }
            let closed = country.coverage.len().saturating_sub(1);
            country.coverage.drain(..closed);
            let closed = country.spending_coverage.len().saturating_sub(1);
            country.spending_coverage.drain(..closed);
        }
        let closed = self.presence.len().saturating_sub(1);
        self.presence.drain(..closed);
    }

    fn compact_open(&mut self, frame: &DecodedFrame) {
        let mut earliest = BTreeMap::new();
        for country in self.countries.values() {
            for day in &country.daily {
                let row = &day.row;
                earliest
                    .entry((row.country, row.instance, row.epoch))
                    .and_modify(|hour: &mut i32| *hour = (*hour).min(row.day_hour))
                    .or_insert(row.day_hour);
            }
            for cost in &country.daily_costs {
                let row = &cost.row;
                earliest
                    .entry((row.country, row.instance, row.epoch))
                    .and_modify(|hour: &mut i32| *hour = (*hour).min(row.day_hour))
                    .or_insert(row.day_hour);
            }
        }
        for (identity, tags) in &mut self.identities {
            let first = tags.first_key_value().map(|(hour, _)| *hour);
            let cutoff = earliest.get(identity).copied().unwrap_or(frame.hour);
            let open = tags
                .range(..=cutoff)
                .next_back()
                .map(|(hour, _)| *hour)
                .unwrap_or(cutoff);
            tags.retain(|hour, _| Some(*hour) == first || *hour >= open);
        }
        let mut hours: BTreeSet<_> = earliest.values().copied().collect();
        hours.insert(frame.hour);
        if let Some(previous) = frame.hour.checked_sub(1) {
            hours.insert(previous);
        }
        self.naming.retain(|(_, hour), _| hours.contains(hour));
        let closed = self.presence.len().saturating_sub(1);
        self.presence.drain(..closed);
        self.holds.retain_open();
        self.shared.retain_open();
    }

    fn prune_publications(&mut self, frame: &DecodedFrame) {
        let _cpu = ::replay::cpu::scope_phase("politics_ledger:pruning");
        // Native change publications advance their sequence cursor within an
        // epoch. Daily costs replace the latest published day. A key removed
        // by a complete publication cannot return; failed reads prove no removal.
        // An hour joined from its changed rows publishes again only the
        // countries whose rows changed; the others publish what they did.
        let mut removed = BTreeSet::new();
        let inputs = Arc::clone(&self.inputs);
        let changed = inputs.changed_rows.as_ref();
        if inputs.power_changed && frame.complete::<tables::CountryPowerChanges>().is_ok() {
            let visits = publish_events(
                &mut self.published_events[0],
                &mut removed,
                changed.map(|changed| &changed[0]),
                |country| {
                    inputs
                        .power_of(country)
                        .map(|row| (row.country, row.instance, row.epoch, row.sequence))
                        .collect()
                },
                || {
                    inputs
                        .power()
                        .iter()
                        .map(|row| (row.country, row.instance, row.epoch, row.sequence))
                        .collect()
                },
            );
            #[cfg(test)]
            {
                self.publication_visits += visits;
            }
            let _ = visits;
        }
        if inputs.governments_changed && frame.complete::<tables::GovernmentChanges>().is_ok() {
            let visits = publish_events(
                &mut self.published_events[1],
                &mut removed,
                changed.map(|changed| &changed[1]),
                |country| {
                    inputs
                        .governments_of(country)
                        .map(|row| (row.country, row.instance, row.epoch, row.sequence))
                        .collect()
                },
                || {
                    inputs
                        .governments()
                        .iter()
                        .map(|row| (row.country, row.instance, row.epoch, row.sequence))
                        .collect()
                },
            );
            #[cfg(test)]
            {
                self.publication_visits += visits;
            }
            let _ = visits;
        }
        for key in removed {
            if !self.published_events[0].contains(&key) && !self.published_events[1].contains(&key)
            {
                self.events.remove(&key);
            }
        }
        if self.inputs.costs_changed
            && frame.complete::<tables::CountryPowerDailyCosts>().is_ok()
            && !self.costs_pruned
        {
            let current = &self.inputs.cost_keys;
            if let Some(owners) = self.inputs.changed_cost_owners() {
                for &country in owners {
                    let old: Vec<_> = self
                        .costs
                        .range(
                            (country, 0, 0, i32::MIN, 0)
                                ..=(country, u32::MAX, u64::MAX, i32::MAX, u32::MAX),
                        )
                        .copied()
                        .collect();
                    #[cfg(test)]
                    {
                        self.cost_prune_visits += old.len();
                    }
                    for key in old {
                        if !current.contains(&key) {
                            self.costs.remove(&key);
                        }
                    }
                }
            } else {
                #[cfg(test)]
                let visits = &mut self.cost_prune_visits;
                self.costs.retain(|key| {
                    #[cfg(test)]
                    {
                        *visits += 1;
                    }
                    current.contains(key)
                });
            }
            self.costs_pruned = true;
        }
    }

    fn note(&mut self, tag: Option<&str>, note: String, count: u64) -> io::Result<()> {
        let key = tag.map(str::to_owned);
        if self.clock.recording {
            count_reason(
                self.clock.warnings.entry(key.clone()).or_default(),
                note.clone(),
                count,
            )?;
        }
        let total = self
            .warnings
            .entry(key)
            .or_default()
            .entry(note)
            .or_default();
        *total = total
            .checked_add(count)
            .ok_or_else(|| io::Error::other("Politics warning count overflow"))?;
        Ok(())
    }
    fn country(&mut self, tag: &str) -> &mut Country {
        self.countries.entry(tag.to_owned()).or_default()
    }
    fn name_at(&self, raw: u32, hour: i32) -> Option<&NamedCountry> {
        self.naming.get(&(raw, hour)).map(Arc::as_ref)
    }
    fn tag_at(&self, country: u32, instance: u32, epoch: u64, hour: i32) -> Option<&str> {
        let tags = self.identities.get(&(country, instance, epoch))?;
        tags.range(..=hour)
            .next_back()
            .or_else(|| tags.first_key_value())
            .map(|(_, tag)| tag.as_str())
    }
    fn name_frame(
        &mut self,
        frame: &DecodedFrame,
        content: Option<&PoliticsContent>,
        requested: &BTreeSet<u32>,
        current: bool,
    ) {
        if requested
            .iter()
            .all(|raw| self.naming.contains_key(&(*raw, frame.hour)))
        {
            return;
        }
        let links = frame.complete::<tables::CountryLinks>().ok();
        let countries = frame.complete::<tables::Countries>().unwrap_or_default();
        let slots: BTreeMap<_, _> = requested
            .iter()
            .filter_map(|raw| {
                let slot = if current {
                    self.inputs.links.get(raw).copied()
                } else {
                    links.and_then(|links| dashboard_text::metadata::link_slot(links, *raw))
                }?;
                let country = if current {
                    let id = self.inputs.naming_ids.get(&slot)?;
                    countries
                        .binary_search_by_key(id, |row| row.id)
                        .ok()
                        .map(|index| &countries[index])
                } else {
                    countries
                        .iter()
                        .rev()
                        .find(|row| row.canonical_slot == slot)
                }?;
                Some((*raw, country))
            })
            .collect();
        let names = content.map(|content| {
            CountryNames::read_selected(
                frame,
                &content.catalogue.localisation,
                slots.values().map(|row| row.canonical_slot),
            )
        });
        for raw in requested {
            if self.naming.contains_key(&(*raw, frame.hour)) {
                continue;
            }
            let Some(country) = slots.get(raw).copied() else {
                continue;
            };
            let Ok(tag) = std::str::from_utf8(&country.tag) else {
                continue;
            };
            if validate_tag(tag).is_err() {
                continue;
            }
            let look = Look::read(frame, country).map(|mut look| {
                look.civil_war = look.civil_war.map_err(|reason| undated_reason(&reason));
                look
            });
            let value = NamedCountry {
                tag: tag.to_owned(),
                name: names
                    .as_ref()
                    .and_then(|names| names.name(country.canonical_slot).ok())
                    .and_then(|name| plain_text(&name).ok()),
                naming: NamingCountry {
                    slot: country.canonical_slot,
                    look,
                },
                ruling_group: names.as_ref().map_or_else(
                    || Err("recorded content unresolved".into()),
                    |names| {
                        names
                            .ruling_group(country.canonical_slot)
                            .map(|group| group.map(str::to_owned))
                            .map_err(|reason| undated_reason(&reason))
                    },
                ),
            };
            let earlier = self
                .naming
                .range((*raw, i32::MIN)..(*raw, frame.hour))
                .next_back();
            let value = match earlier {
                Some((_, old)) if **old == value => Arc::clone(old),
                _ => Arc::new(value),
            };
            self.naming.insert((*raw, frame.hour), value);
        }
    }
    fn refresh_modifier_formats(&mut self, frame: &DecodedFrame) {
        if let (Ok(rows), Ok(tokens)) = (
            frame.complete::<tables::ModifierDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            let definitions = &frame.tables[TableId::ModifierDefinitions.index()];
            let lexical = &frame.tables[TableId::LexicalTokens.index()];
            let same =
                self.modifier_views
                    .as_ref()
                    .is_some_and(|(old_definitions, old_lexical)| {
                        old_definitions.is(definitions) && old_lexical.is(lexical)
                    });
            if !same {
                self.modifier_definitions = Some(idea_format::index_recorded(rows, tokens));
                self.modifier_views = Some((definitions.identity(), lexical.identity()));
            }
        }
    }
}
/// A read the next read dates no row at, as that read uses it: to name
/// countries (`CountryNames`, `Look::read`) and government tokens. Its other
/// tables share one unavailable view: holding them would copy each one the
/// decoder changes for the next read.
fn naming_frame(frame: &DecodedFrame) -> DecodedFrame {
    use replay::frame::{SharedView, TABLE_COUNT, TableView};
    // The civil-war operands of a country's look, then government token spellings.
    let read = [
        TableId::CountryCivilWars,
        TableId::CountryCounterInputs,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::LexicalTokens,
    ];
    let unread = SharedView::from(TableView::Unavailable {
        reason: "not retained after its read".into(),
    });
    let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| unread.clone());
    for table in installed_content::country_names::TABLES.into_iter().chain(read) {
        tables[table.index()] = frame.tables[table.index()].clone();
    }
    DecodedFrame {
        branch: frame.branch,
        hour: frame.hour,
        tables,
        metrics: frame.metrics.clone(),
    }
}

type EventKey = (u32, u32, u64, u64);

/// Replace a table's published event keys with this read's, collecting those
/// it no longer publishes: those of the `changed` countries only, when only
/// their rows changed, otherwise every key. The keys visited.
fn publish_events(
    published: &mut BTreeSet<EventKey>,
    removed: &mut BTreeSet<EventKey>,
    changed: Option<&BTreeSet<u32>>,
    current_of: impl Fn(u32) -> BTreeSet<EventKey>,
    all: impl FnOnce() -> BTreeSet<EventKey>,
) -> usize {
    let Some(changed) = changed else {
        let current = all();
        let visits = current.len() + published.len();
        removed.extend(published.difference(&current).copied());
        *published = current;
        return visits;
    };
    let mut visits = 0;
    for &country in changed {
        let current = current_of(country);
        let old: Vec<_> = published
            .range((country, 0, 0, 0)..=(country, u32::MAX, u64::MAX, u64::MAX))
            .copied()
            .collect();
        visits += current.len() + old.len();
        for key in old {
            if !current.contains(&key) {
                published.remove(&key);
                removed.insert(key);
            }
        }
        published.extend(current);
    }
    visits
}

/// Whether this read observes a power or cost row dated at `hour` that is not
/// yet recorded and names a known country.
fn dated_at(scan: &Scanned, native: &inputs::Inputs, hour: i32) -> bool {
    let power = (native.power_changed || native.identity_changed)
        && native.power_rows().any(|row| {
            row.hour == hour
                && !scan
                    .events
                    .contains(&(row.country, row.instance, row.epoch, row.sequence))
                && scan
                    .tag_at(row.country, row.instance, row.epoch, row.hour)
                    .is_some()
        });
    let costs = native.costs_changed || native.identity_changed;
    power
        || costs
            && native.cost_rows().any(|row| {
                row.day_hour == hour
                    && !scan.costs.contains(&(
                        row.country,
                        row.instance,
                        row.epoch,
                        row.day_hour,
                        row.ordinal,
                    ))
                    && scan
                        .tag_at(row.country, row.instance, row.epoch, row.day_hour)
                        .is_some()
            })
}

/// A read decoded again from the recording, every table as preparation
/// decodes it.
fn read_again(replay: &replay::container::Replay, read: &DecodedFrame) -> io::Result<DecodedFrame> {
    match replay.seek(read.branch, read.hour)? {
        replay::container::SeekResult::Frame(frame) => Ok(*frame),
        replay::container::SeekResult::Gap { reason, .. } => Err(dashboard_core::invalid(format!(
            "Politics ledger could not read hour {} again: {reason}",
            read.hour
        ))),
    }
}

fn merge_coverage(rows: &mut Vec<LedgerCoverage>, value: LedgerCoverage) {
    if let Some(old) = rows.last_mut()
        && old.country == value.country
        && old.instance == value.instance
        && old.epoch == value.epoch
        && old.streams == value.streams
        && old.reason == value.reason
        && old.through_hour.and_then(|hour| hour.checked_add(1)) == value.through_hour
        && old
            .through_hour
            .zip(value.since_hour)
            .is_some_and(|(last, first)| first <= last.saturating_add(1))
    {
        old.through_hour = value.through_hour;
    } else {
        rows.push(value);
    }
}
fn merge_spending_coverage(rows: &mut Vec<LedgerSpendCoverage>, value: LedgerSpendCoverage) {
    if let Some(old) = rows.last_mut()
        && old.political == value.political
        && old.command == value.command
        && old.reason == value.reason
        && old.through_hour.and_then(|hour| hour.checked_add(1)) == value.since_hour
    {
        old.through_hour = value.through_hour;
    } else {
        rows.push(value);
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct PresenceRun {
    first: i32,
    last: i32,
    tags: BTreeSet<String>,
    refused_tags: BTreeSet<String>,
}
// Tags accept three non-NUL bytes, without a UTF-8 check.
// Refuse only the unrequestable identity, preserving other countries' facts.
fn valid_recorded_tag(country: &CountryRow) -> Result<&str, String> {
    let reason = match std::str::from_utf8(&country.tag) {
        Err(_) => Some("not UTF-8"),
        Ok(tag) => validate_tag(tag).err().map(|_| {
            if tag.is_empty() {
                "empty"
            } else if tag.contains('\0') {
                "contains NUL"
            } else {
                "exceeds shared country-tag length bound"
            }
        }),
    };
    if let Some(reason) = reason {
        let escaped: String = country
            .tag
            .iter()
            .take(32)
            .flat_map(|byte| std::ascii::escape_default(*byte))
            .map(char::from)
            .collect();
        return Err(format!(
            "country {} canonical slot {}: recorded tag b\"{}\"{} ({} bytes), {reason}",
            country.id,
            country.canonical_slot,
            escaped,
            if country.tag.len() > 32 {
                " (prefix)"
            } else {
                ""
            },
            country.tag.len()
        ));
    }
    Ok(std::str::from_utf8(&country.tag).expect("validated UTF-8"))
}
fn observe_presence(scan: &mut Scanned, frame: &DecodedFrame) -> io::Result<()> {
    if frame.complete::<tables::Countries>().is_err() {
        return Ok(());
    }
    let inputs = Arc::clone(&scan.inputs);
    if scan.clock.recording
        && !inputs.countries_changed
        && scan.clock.previous.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
    {
        if let Some(last) = scan
            .presence
            .last_mut()
            .filter(|last| last.last.checked_add(1) == Some(frame.hour))
        {
            last.last = frame.hour;
        }
        for tag in scan.clock.selected.clone() {
            if let Some(reasons) = scan.clock.tag_refusals.get(&tag).cloned() {
                for (reason, count) in reasons {
                    scan.note(Some(&tag), reason, count)?;
                }
            }
        }
        return Ok(());
    }
    if scan.clock.recording {
        scan.clock.presence_refreshed = true;
        scan.clock.tag_refusals.clear();
    }
    let countries = inputs.census.iter();
    let mut tags = BTreeSet::new();
    let mut refused_tags = BTreeSet::new();
    for country in countries {
        match valid_recorded_tag(country) {
            Ok(tag) => {
                tags.insert(tag);
                if scan
                    .known_tags
                    .get(&country.canonical_slot)
                    .is_none_or(|old| old != tag)
                {
                    scan.known_tags
                        .insert(country.canonical_slot, tag.to_owned());
                }
            }
            Err(reason) => {
                if let Some(tag) = scan.known_tags.get(&country.canonical_slot).cloned() {
                    refused_tags.insert(tag.clone());
                    if scan.clock.recording {
                        *scan
                            .clock
                            .tag_refusals
                            .entry(tag.clone())
                            .or_default()
                            .entry(reason.clone())
                            .or_default() += 1;
                    }
                    scan.note(Some(&tag), reason, 1)?;
                }
            }
        }
    }
    // An unchanged census extends its run without owning the hour's tags.
    if let Some(last) = scan.presence.last_mut()
        && last.last.checked_add(1) == Some(frame.hour)
        && last.tags.iter().eq(&tags)
        && last.refused_tags == refused_tags
    {
        last.last = frame.hour;
    } else {
        scan.presence.push(PresenceRun {
            first: frame.hour,
            last: frame.hour,
            tags: tags.into_iter().map(str::to_owned).collect(),
            refused_tags,
        });
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn observe_all(
    frame: &DecodedFrame,
    scan: &mut Scanned,
    content: Option<&PoliticsContent>,
) -> io::Result<()> {
    Arc::make_mut(&mut scan.inputs).refresh(frame, None);
    scan.costs_pruned = false;
    observe_accounting(frame, scan, content)
}

fn observe_accounting(
    frame: &DecodedFrame,
    scan: &mut Scanned,
    content: Option<&PoliticsContent>,
) -> io::Result<()> {
    let native = Arc::clone(&scan.inputs);
    let hour = frame.hour;
    observe_presence(scan, frame)?;
    scan.refresh_modifier_formats(frame);
    let mut event_countries = BTreeSet::new();
    for row in native.power_rows() {
        if !scan
            .events
            .contains(&(row.country, row.instance, row.epoch, row.sequence))
        {
            event_countries.insert(row.country);
            event_countries.extend(row.identity.target_country);
        }
    }
    for row in native.cost_rows() {
        if !scan.costs.contains(&(
            row.country,
            row.instance,
            row.epoch,
            row.day_hour,
            row.ordinal,
        )) {
            event_countries.insert(row.country);
            event_countries.extend(row.identity.target_country);
        }
    }
    scan.name_frame(frame, content, &event_countries, true);
    if let Some(previous) = scan.previous.take() {
        scan.name_frame(&previous, content, &event_countries, false);
        scan.previous = Some(previous);
    }
    for country in native.refused.iter().take(
        if !scan.clock.recording
            || native.countries_changed
            || scan.clock.previous.and_then(|hour| hour.checked_add(1)) != Some(hour)
        {
            native.refused.len()
        } else {
            0
        },
    ) {
        if let Err(reason) = valid_recorded_tag(country) {
            let entry = scan
                .refused
                .entry(country.canonical_slot)
                .or_insert((reason, 0));
            entry.1 = entry
                .1
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Refused country count overflow"))?;
        }
    }
    let countries = &native.countries;
    let links = &native.links;
    let witnesses = &native.witnesses;
    let by_slot = &native.by_slot;
    let daily_mutations = &native.daily_mutations;
    let unknown_additions = &native.unknown_additions;
    for witness in native.identity_rows() {
        if let Some(country) = links
            .get(&witness.country)
            .and_then(|slot| countries.get(slot))
        {
            let tag = std::str::from_utf8(&country.tag)
                .expect("recorded country tags validated at scan entry");
            let tags = scan
                .identities
                .entry((witness.country, witness.instance, witness.epoch))
                .or_default();
            if tags.last_key_value().is_none_or(|(_, old)| old != tag) {
                tags.insert(hour, tag.to_owned());
            }
        }
    }
    let applications = &native.applications;
    let mut unmatched = BTreeSet::new();
    if native.power_changed || native.identity_changed {
        for event in native.power_rows() {
            if event.cause == PowerCause::DailyGain
                && event.currency == PowerCurrency::Political
                && !scan.events.contains(&(
                    event.country,
                    event.instance,
                    event.epoch,
                    event.sequence,
                ))
                && !applications.contains(&(
                    event.country,
                    event.instance,
                    event.epoch,
                    event.hour,
                    event.sequence,
                ))
            {
                unmatched.insert(event.country);
            }
        }
    }
    let unreconciled =
        |country: &u32| native.unreconciled.contains(country) || unmatched.contains(country);
    if scan.clock.recording {
        scan.clock
            .recheck
            .extend(unmatched.iter().filter_map(|raw| links.get(raw)).copied());
    }
    let mut unmatched_counts = BTreeMap::<u32, u64>::new();
    for raw in unmatched
        .iter()
        .filter(|raw| !native.unreconciled.contains(raw))
    {
        if let Some(slot) = links.get(raw) {
            *unmatched_counts.entry(*slot).or_default() += 1;
        }
    }
    // Every current country gets a witness or an explicit unknown interval.
    let diagnostic_slots = if scan.clock.recording {
        scan.clock
            .selected
            .iter()
            .filter_map(|tag| native.tag_slots.get(tag))
            .flatten()
            .copied()
            .collect()
    } else {
        native.slots.clone()
    };
    for country in diagnostic_slots
        .iter()
        .filter_map(|slot| countries.get(slot))
    {
        let tag = std::str::from_utf8(&country.tag)
            .expect("recorded country tags validated at scan entry");
        #[cfg(test)]
        {
            scan.country_visits += 1;
        }
        let witness = by_slot.get(&country.canonical_slot);
        let value = witness.map_or_else(
            || LedgerCoverage {
                country: Some(country.id),
                instance: None,
                read_hour: hour,
                since_hour: Some(hour),
                through_hour: Some(hour),
                epoch: None,
                streams: 0,
                reason: Some("country identity or politics coverage unavailable".into()),
            },
            |row| coverage(row, hour),
        );
        let value = if value.through_hour != Some(hour) {
            LedgerCoverage {
                streams: 0,
                since_hour: Some(hour),
                through_hour: Some(hour),
                reason: Some("politics coverage is stale at this read".into()),
                ..value
            }
        } else {
            value
        };
        let changes_known = frame.complete::<tables::CountryPowerChanges>().is_ok();
        let daily_known = frame.complete::<tables::CountryPowerDaily>().is_ok()
            && frame.complete::<tables::CountryPowerDailyCosts>().is_ok();
        let raw = witness.map_or(country.id, |row| row.country);
        let has_daily = daily_mutations.get(&raw).is_some_and(|hours| {
            hours
                .range(value.since_hour.unwrap_or(hour)..=hour)
                .next()
                .is_some()
        });
        let unknown_addition = witness.map_or(0, |witness| {
            unknown_additions
                .get(&witness.country)
                .and_then(|values| values.get(&(witness.instance, witness.epoch)))
                .copied()
                .unwrap_or(0)
        });
        let political = value.streams & PoliticsCaptureCoverageRow::PP != 0
            && changes_known
            && daily_known
            && !unreconciled(&raw)
            && (!has_daily || value.streams & PoliticsCaptureCoverageRow::PP_DAILY_COSTS != 0)
            && unknown_addition & PoliticsCaptureCoverageRow::PP == 0;
        let command = value.streams & PoliticsCaptureCoverageRow::CP != 0
            && changes_known
            && unknown_addition & PoliticsCaptureCoverageRow::CP == 0;
        let spending_cover = LedgerSpendCoverage {
            read_hour: hour,
            since_hour: Some(hour),
            through_hour: Some(hour),
            political,
            command,
            reason: if !changes_known {
                Some("Spending events were not recorded at this read".into())
            } else if !daily_known
                || unreconciled(&raw)
                || has_daily && value.streams & PoliticsCaptureCoverageRow::PP_DAILY_COSTS == 0
            {
                Some("Daily political power costs were not recorded at this read".into())
            } else if unknown_addition != 0 {
                Some(match unknown_addition {
                    PoliticsCaptureCoverageRow::PP => "A political power adjustment has no recorded evaluated operand",
                    PoliticsCaptureCoverageRow::CP => "A command power adjustment has no recorded evaluated operand",
                    _ => "Political and command power adjustments have no recorded evaluated operands",
                }.into())
            } else {
                value.reason.clone()
            },
        };
        let history = scan.country(tag);
        let previous = (history.coverage.len(), history.spending_coverage.len());
        merge_coverage(&mut history.coverage, value);
        merge_spending_coverage(&mut history.spending_coverage, spending_cover);
        if previous != (history.coverage.len(), history.spending_coverage.len()) {
            scan.dirty.insert(tag.to_owned());
        }
    }
    for country in diagnostic_slots
        .iter()
        .filter_map(|slot| countries.get(slot))
    {
        let steady = native
            .unreconciled_counts
            .get(&country.canonical_slot)
            .copied()
            .unwrap_or(0);
        let transient = unmatched_counts
            .get(&country.canonical_slot)
            .copied()
            .unwrap_or(0);
        let reason =
            "Daily PP rates/components have no matching native application and wrapping sum";
        if steady != 0 {
            scan.note(valid_recorded_tag(country).ok(), reason.into(), steady)?;
        }
        if transient != 0 {
            if scan.clock.recording {
                // A newly observed application without its tuple warns once;
                // retained unresolved daily tuples warn on each accepted hour.
                count_reason(
                    scan.warnings
                        .entry(valid_recorded_tag(country).ok().map(str::to_owned))
                        .or_default(),
                    reason.into(),
                    transient,
                )?;
            } else {
                scan.note(valid_recorded_tag(country).ok(), reason.into(), transient)?;
            }
        }
    }
    for table in [
        TableId::CountryPowerChanges,
        TableId::GovernmentChanges,
        TableId::CountryPowerDaily,
        TableId::CountryPowerDailyCosts,
    ] {
        if !matches!(
            &*frame.tables[table.index()],
            replay::frame::TableView::Complete(_)
        ) {
            for country in diagnostic_slots
                .iter()
                .filter_map(|slot| countries.get(slot))
            {
                scan.note(
                    valid_recorded_tag(country).ok(),
                    format!("{table:?} unavailable in part of this branch"),
                    1,
                )?;
            }
        }
    }
    let covered = |country, instance, epoch, event_hour, bit| {
        witnesses.get(&country).is_some_and(|row| {
            row.instance == instance
                && row.epoch == epoch
                && row.through_hour == hour
                && row.streams & bit == bit
                && event_hour >= row.since_hour
                && event_hour <= row.through_hour
        })
    };
    let mut previous = scan.previous.take();
    // A row dated at a previous read held only by its naming tables is
    // described from that read decoded again.
    let again = match &previous {
        Some(read)
            if !scan.previous_whole && read.hour != hour && dated_at(scan, &native, read.hour) =>
        {
            scan.replay
                .as_ref()
                .map(|replay| read_again(replay, read))
                .transpose()?
        }
        _ => None,
    };
    if again.is_some() {
        #[cfg(test)]
        {
            scan.reads_again += 1;
        }
        previous = again;
    }
    let current_texts = FrameTexts::default();
    let previous_texts = FrameTexts::default();
    if native.power_changed || native.identity_changed {
        for row in native.power_rows() {
            // A row a later read publishes again is skipped before its tag.
            let key = (row.country, row.instance, row.epoch, row.sequence);
            if scan.events.contains(&key) {
                continue;
            }
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.hour)
                .map(str::to_owned)
            else {
                continue;
            };
            scan.events.insert(key);
            let bit = match row.currency {
                PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
                PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
            };
            let mut mutation = Mutation {
                row: row.clone(),
                read_hour: hour,
                covered: covered(row.country, row.instance, row.epoch, row.hour, bit),
                facts: EventFacts::default(),
            };
            {
                let event_frame = if row.hour == hour {
                    Some(frame)
                } else {
                    previous
                        .as_ref()
                        .filter(|previous| previous.hour == row.hour)
                };
                let texts = if row.hour == hour {
                    &current_texts
                } else {
                    &previous_texts
                };
                let ledger = mutation.ledger();
                mutation.facts =
                    event_facts(&mut scan.shared, &ledger, event_frame, row.cause, None);
                if let Some(content) = content {
                    mutation.facts.text =
                        item_text(scan, &ledger, &mutation.facts, content, event_frame, texts);
                }
            }
            scan.dirty.insert(tag.clone());
            scan.country(&tag).mutations.push(mutation);
        }
    }
    if native.governments_changed || native.identity_changed {
        for row in native.government_rows() {
            let key = (row.country, row.instance, row.epoch, row.sequence);
            if scan.events.contains(&key) {
                continue;
            }
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.hour)
                .map(str::to_owned)
            else {
                continue;
            };
            scan.events.insert(key);
            let before_token =
                government_token(frame, previous.as_ref(), row.hour, row.before_ideology);
            let after_token =
                government_token(frame, previous.as_ref(), row.hour, row.after_ideology);
            scan.dirty.insert(tag.clone());
            scan.country(&tag).governments.push(LedgerGovernment {
                country: row.country,
                instance: row.instance,
                sequence: row.sequence.to_string(),
                epoch: row.epoch.to_string(),
                hour: row.hour,
                read_hour: hour,
                cause: format!("{:?}", row.cause),
                before_party: row.before_party,
                after_party: row.after_party,
                before_ideology: row.before_ideology,
                after_ideology: row.after_ideology,
                before_token,
                after_token,
                actor_country: row.actor_country,
                origin_kind: format!("{:?}", row.origin_kind),
                origin_token: row.origin_token,
                origin_key: row.origin_key.clone(),
                covered: covered(
                    row.country,
                    row.instance,
                    row.epoch,
                    row.hour,
                    PoliticsCaptureCoverageRow::GOVERNMENT,
                ),
            });
        }
    }
    if native.daily_changed || native.identity_changed {
        for row in native.daily_rows() {
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.day_hour)
                .map(str::to_owned)
            else {
                continue;
            };
            let key = (row.country, row.instance, row.epoch, row.day_hour);
            let mut value = Daily {
                row: row.clone(),
                read_hour: hour,
                pp_read_hour: [
                    row.pp_gross_gain_raw,
                    row.pp_cost_raw,
                    row.pp_net_gain_raw,
                    row.pp_realized_delta_raw,
                ]
                .iter()
                .any(Option::is_some)
                .then_some(hour)
                .or(row.pp_sequence.map(|_| hour)),
                cp_read_hour: [row.cp_gain_raw, row.cp_realized_delta_raw]
                    .iter()
                    .any(Option::is_some)
                    .then_some(hour)
                    .or(row.cp_sequence.map(|_| hour)),
            };
            if let Some(&index) = scan.daily.get(&key) {
                let old = &mut scan.country(&tag).daily[index];
                // publish_daily commits matching PP rate/application operands
                // together, independently of CP.
                if (
                    row.pp_gross_gain_raw,
                    row.pp_cost_raw,
                    row.pp_net_gain_raw,
                    row.pp_realized_delta_raw,
                    row.pp_sequence,
                ) == (
                    old.row.pp_gross_gain_raw,
                    old.row.pp_cost_raw,
                    old.row.pp_net_gain_raw,
                    old.row.pp_realized_delta_raw,
                    old.row.pp_sequence,
                ) {
                    value.pp_read_hour = old.pp_read_hour;
                }
                if (row.cp_gain_raw, row.cp_realized_delta_raw, row.cp_sequence)
                    == (
                        old.row.cp_gain_raw,
                        old.row.cp_realized_delta_raw,
                        old.row.cp_sequence,
                    )
                {
                    value.cp_read_hour = old.cp_read_hour;
                }
                let mut unchanged = value.clone();
                unchanged.read_hour = old.read_hour;
                if *old != unchanged {
                    *old = value;
                    scan.dirty.insert(tag.clone());
                }
            } else {
                let index = scan.country(&tag).daily.len();
                scan.daily.insert(key, index);
                scan.country(&tag).daily.push(value);
                scan.dirty.insert(tag.clone());
            }
        }
    }
    if native.costs_changed || native.identity_changed {
        for row in native.cost_rows() {
            let key = (
                row.country,
                row.instance,
                row.epoch,
                row.day_hour,
                row.ordinal,
            );
            if scan.costs.contains(&key) {
                continue;
            }
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.day_hour)
                .map(str::to_owned)
            else {
                continue;
            };
            scan.costs.insert(key);
            let mut cost = DailyCost {
                row: row.clone(),
                read_hour: hour,
                covered: covered(
                    row.country,
                    row.instance,
                    row.epoch,
                    row.day_hour,
                    PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::PP_DAILY_COSTS,
                ) && !unreconciled(&row.country),
                facts: EventFacts::default(),
            };
            {
                let event_frame = if row.day_hour == hour {
                    Some(frame)
                } else {
                    previous
                        .as_ref()
                        .filter(|previous| previous.hour == row.day_hour)
                };
                let texts = if row.day_hour == hour {
                    &current_texts
                } else {
                    &previous_texts
                };
                let mutation = daily_mutation(&cost.ledger());
                cost.facts = event_facts(
                    &mut scan.shared,
                    &mutation,
                    event_frame,
                    row.cause,
                    Some(row.component),
                );
                if let Some(content) = content {
                    cost.facts.text =
                        item_text(scan, &mutation, &cost.facts, content, event_frame, texts);
                }
            }
            scan.dirty.insert(tag.clone());
            scan.country(&tag).daily_costs.push(cost);
        }
    }
    holds::observe_holds(frame, scan, content);
    scan.first_hour.get_or_insert(hour);
    scan.last_hour = Some(scan.last_hour.map_or(hour, |last| last.max(hour)));
    scan.retain_previous(frame);
    Ok(())
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
// Native categories are capture provenance, not a content-price catalogue.
fn category(cause: PowerCause) -> (&'static str, &'static str) {
    match cause {
        PowerCause::Idea => ("ideas", "Ideas"),
        PowerCause::Advisor | PowerCause::AdvisorRemoval => ("advisors", "Advisors"),
        PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout => {
            ("decisions", "Decisions")
        }
        PowerCause::Diplomacy | PowerCause::JoinWar => ("diplomacy", "Diplomacy"),
        PowerCause::WarJustification => ("warJustification", "War justification"),
        PowerCause::Focus | PowerCause::FocusRefund => ("focus", "Focus"),
        PowerCause::ScriptEffect | PowerCause::ScaledScriptEffect => {
            ("scriptEffects", "Script effects")
        }
        PowerCause::Autonomy => ("autonomy", "Autonomy"),
        PowerCause::SpyMaster => ("spyMaster", "Spy master"),
        PowerCause::ScientistRecruitment | PowerCause::ScientistFaction => {
            ("scientists", "Scientists")
        }
        PowerCause::ScorchedEarth => ("scorchedEarth", "Scorched earth"),
        PowerCause::PrideOfFleet => ("prideOfFleet", "Pride of the fleet"),
        PowerCause::Blockade => ("blockade", "Blockade"),
        PowerCause::MioPolicy => ("mioPolicy", "Organisation policies"),
        PowerCause::Medal => ("medals", "Medals"),
        PowerCause::Console => ("console", "Console"),
        PowerCause::ResetSetup => ("setup", "Setup"),
        PowerCause::AbilityActivation => ("abilities", "Abilities"),
        PowerCause::AbilityUpkeep => ("upkeep", "Upkeep"),
        PowerCause::Allocation | PowerCause::AllocationRefund => ("allocation", "Allocation"),
        PowerCause::LeaderTrait => ("traits", "Traits"),
        PowerCause::LeaderPromotion => ("promotions", "Promotions"),
        PowerCause::LeaderAssignment => ("assignments", "Assignments"),
        PowerCause::LeaderRecruitment => ("recruitment", "Recruitment"),
        PowerCause::Raid => ("raids", "Raids"),
        PowerCause::AirAction => ("airActions", "Air actions"),
        PowerCause::PreferredTactic => ("tactics", "Preferred tactics"),
        PowerCause::OfficerUpgrade => ("officerUpgrades", "Officer upgrades"),
        PowerCause::CaptainUpgrade => ("captainUpgrades", "Captain upgrades"),
        PowerCause::TimedActivity => ("timedActivities", "Timed activities"),
        PowerCause::DailyGain => ("dailyDeficit", "Daily deficit"),
        PowerCause::CapCorrection => ("capCorrection", "Cap correction"),
        PowerCause::NotIdentified => ("notIdentified", "Not identified"),
    }
}
/// The capture normalizes evaluated charges to signed deltas. Do not
/// classify from the realized delta: a no-deficit/cap floor may swallow it.
fn charge(row: &LedgerMutation) -> Option<Option<i128>> {
    // PP's net daily application includes its separately recorded costs.
    // CP instead applies the signed daily operand without clamping
    // it nonnegative.
    if holds::is_hold(row)
        || row.cause == "DailyGain" && row.currency == "Political"
        || !matches!(row.operation.as_str(), "Add" | "Subtract")
    {
        return None;
    }
    match row
        .amount_raw
        .as_deref()
        .and_then(|raw| raw.parse::<i128>().ok())
    {
        Some(raw) if raw < 0 => Some(Some(-raw)),
        Some(_) => None,
        None if row.operation == "Subtract" => Some(None),
        None => None,
    }
}

fn recorded_token<'a>(facts: &'a EventFacts, row: &'a LedgerMutation) -> Option<&'a str> {
    facts.token.as_deref().or_else(|| {
        (!row.item.key.is_empty())
            .then(|| std::str::from_utf8(&row.item.key).ok())
            .flatten()
    })
}
fn mutation_id(row: &LedgerMutation) -> String {
    format!(
        "{}:{}:{}:{}",
        row.country, row.instance, row.epoch, row.sequence
    )
}
fn daily_mutation(cost: &LedgerDailyCost) -> LedgerMutation {
    LedgerMutation {
        country: cost.country,
        instance: cost.instance,
        sequence: cost.pp_sequence.clone(),
        epoch: cost.epoch.clone(),
        hour: cost.day_hour,
        read_hour: cost.read_hour,
        currency: "Political".into(),
        cause: cost.cause.clone(),
        operation: "Subtract".into(),
        site: format!("daily component:{}", cost.ordinal),
        amount_raw: cost
            .amount_raw
            .parse::<i128>()
            .ok()
            .and_then(i128::checked_neg)
            .map(|raw| raw.to_string()),
        realized_delta_raw: String::new(),
        before_raw: String::new(),
        after_raw: String::new(),
        item: cost.item.clone(),
        covered: cost.covered,
    }
}
/// What resolving an hour's item texts builds once: the trigger world, and the
/// scripted-text context of each root country (each reads every country's names
/// and scopes from the frame).
#[derive(Default)]
struct FrameTexts<'a> {
    world: std::sync::OnceLock<Result<dashboard_text::triggers::World<'a>, String>>,
    scripted: std::cell::RefCell<
        BTreeMap<u32, std::rc::Rc<dashboard_text::scripted_text::ScriptedText<'a>>>,
    >,
}

impl<'a> FrameTexts<'a> {
    /// The hour's scripted-text context for `root`, built once.
    fn scripted(
        &self,
        content: &'a PoliticsContent,
        frame: &'a DecodedFrame,
        root: u32,
    ) -> Result<std::rc::Rc<dashboard_text::scripted_text::ScriptedText<'a>>, String> {
        if let Some(scripted) = self.scripted.borrow().get(&root) {
            return Ok(std::rc::Rc::clone(scripted));
        }
        let scripted = std::rc::Rc::new(dashboard_text::scripted_text::ScriptedText::new(
            frame,
            &content.catalogue.localisation,
            content.catalogue.scripted_content()?,
            root,
        ));
        self.scripted
            .borrow_mut()
            .insert(root, std::rc::Rc::clone(&scripted));
        Ok(scripted)
    }
}

fn item_text<'a>(
    scan: &mut Scanned,
    row: &LedgerMutation,
    facts: &EventFacts,
    content: &'a PoliticsContent,
    frame: Option<&'a DecodedFrame>,
    texts: &FrameTexts<'a>,
) -> Option<Arc<ItemText>> {
    let mut text = event_text(scan, row, facts, content, frame, texts)?;
    text.unknown = text.unknown.map(|reason| undated_reason(&reason));
    Some(scan.shared.text(text))
}

fn localise_at<'a>(
    content: &'a PoliticsContent,
    key: &str,
    frame: Option<&'a DecodedFrame>,
    texts: &FrameTexts<'a>,
    slot: Option<u32>,
    decision: Option<(&LedgerMutation, bool)>,
    unknown_seed: bool,
) -> Result<Option<String>, String> {
    if !content.catalogue.has_presentation_key(key) {
        return Err(format!(
            "localisation {key} is missing; title is unavailable"
        ));
    }
    let raw = content
        .catalogue
        .localisation
        .localise(key.as_bytes())
        .map_err(|error| error.to_string())?;
    if raw.contains('[') {
        let slot = slot.ok_or_else(|| {
            format!("localisation {key}: country identity is unavailable at action hour")
        })?;
        let frame = frame
            .ok_or_else(|| format!("localisation {key}: its event-hour frame is unavailable"))?;
        let scripted = texts.scripted(content, frame, slot)?;
        if let Some((decision, selects_state)) = decision {
            use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
            // The recorded identity supplies the country target, but not
            // the name seed or conditional state-target selector.
            let mut root = EventScope::of(EventObject::Country(slot));
            if selects_state {
                root.from = Link::Unknown;
            }
            let mut scopes = EventScopes::new(vec![root]);
            let mut unknown_seeds = BTreeSet::from([0]);
            if let Some(raw) = decision.item.target_country {
                let target = frame
                    .complete::<tables::CountryLinks>()
                    .ok()
                    .and_then(|rows| {
                        rows.binary_search_by_key(&raw, |row| row.id)
                            .ok()
                            .map(|index| rows[index].canonical_slot)
                    });
                if let Some(target) = target {
                    scopes.scopes[0].from = Link::Scope(1);
                    let mut from = EventScope::of(EventObject::Country(target));
                    if selects_state {
                        from.from = Link::Unknown;
                    }
                    scopes.scopes.push(from);
                    unknown_seeds.insert(1);
                } else {
                    scopes.scopes[0].from = Link::Unknown;
                }
            } else if let Some(state) = decision.item.target_state {
                let present = state > 0
                    && match &*frame.tables[TableId::States.index()] {
                        replay::frame::TableView::Complete(rows)
                        | replay::frame::TableView::Truncated { rows, .. } => {
                            rows.rows::<tables::States>().is_some_and(|states| {
                                states.binary_search_by_key(&state, |row| row.id).is_ok()
                            })
                        }
                        replay::frame::TableView::Unavailable { .. } => false,
                    };
                if present {
                    scopes.scopes[0].from = Link::Scope(1);
                    let mut from = EventScope::of(EventObject::State(state));
                    // An existing positive FROM state takes precedence over selector
                    // arrays/triggers. It becomes a fresh nested state scope with
                    // default seed 1.
                    if selects_state {
                        from.from = Link::Scope(2);
                    }
                    scopes.scopes.push(from);
                    unknown_seeds.insert(1);
                    if selects_state {
                        scopes
                            .scopes
                            .push(EventScope::of(EventObject::State(state)));
                    }
                } else {
                    scopes.scopes[0].from = Link::Unknown;
                }
            } else if decision.cause == "TargetedDecision" {
                scopes.scopes[0].from = Link::Unknown;
            }
            for index in unknown_seeds {
                scopes.scopes[index].random_seed_refusal =
                    Some("decision caller random seed is not recorded");
            }
            return scripted
                .expand_in(&raw, &scopes)
                .and_then(|text| plain_text(&text))
                .map(Some);
        }
        if unknown_seed {
            use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes};
            let mut scope = EventScope::of(EventObject::Country(slot));
            scope.random_seed_refusal = Some("focus caller random seed is not recorded");
            let scopes = EventScopes::new(vec![scope]);
            return scripted
                .expand_in(&raw, &scopes)
                .and_then(|text| plain_text(&text))
                .map(Some);
        }
        Ok(content.catalogue.localise_in(key, &scripted))
    } else {
        Ok(content.localised(key, View::Scripted))
    }
}
fn event_world<'a>(
    content: &'a PoliticsContent,
    frame: &'a DecodedFrame,
) -> Result<dashboard_text::triggers::World<'a>, String> {
    let full = content
        .catalogue
        .scripted_content()
        .and_then(|scripted| scripted.recorded_world(frame));
    match full {
        Ok(world) => Ok(world),
        Err(_) => {
            let power = content.power_content()?;
            Ok(dashboard_text::triggers::World::read(
                frame,
                dashboard_text::triggers::ContentFacts::content_only(
                    &power.dlcs,
                    &power.triggers,
                    &power.aliases,
                ),
            ))
        }
    }
}
/// Resolve only text and sprite identity while the exact event frame is still
/// present; PNGs are decoded later for the country actually requested. Covered
/// batches span adjacent reads. A late event outside those frames retains its
/// recorded item/price but cannot borrow a later trigger state.
fn event_text<'a>(
    scan: &Scanned,
    row: &LedgerMutation,
    facts: &EventFacts,
    content: &'a PoliticsContent,
    frame: Option<&'a DecodedFrame>,
    texts: &FrameTexts<'a>,
) -> Option<ItemText> {
    let own = scan.name_at(row.country, row.hour);
    let mut found = ItemText::default();
    if row.cause == "DailyGain" && row.item.object_kind == "Organisation" {
        let result = (|| -> Result<String, String> {
            let reference = row
                .item
                .object_reference
                .as_deref()
                .and_then(|reference| reference.parse().ok())
                .ok_or_else(|| {
                    "daily organisation cost: recorded reference is unavailable".to_owned()
                })?;
            let frame = frame
                .filter(|frame| frame.hour == row.hour)
                .ok_or_else(|| {
                    format!("organisation {reference:#x}: its action-hour frame is unavailable")
                })?;
            let root = own.map_or(0, |own| own.naming.slot);
            // The name never depends on the root unless its text is scripted, so
            // one context per hour serves every country's plain names.
            let any = texts
                .scripted
                .borrow()
                .keys()
                .next()
                .copied()
                .unwrap_or(root);
            let (template, scopes) = texts
                .scripted(content, frame, any)?
                .organisation_template(reference)?;
            let name = if template.contains('[') {
                texts
                    .scripted(content, frame, root)?
                    .expand_in(&template, &scopes)?
            } else {
                template
            };
            plain_text(&name)
        })();
        match result {
            Ok(name) => found.name = Some(name),
            Err(reason) => found.unknown = Some(reason),
        }
        return Some(found);
    }
    let token = recorded_token(facts, row)?;
    if matches!(
        row.cause.as_str(),
        "Decision" | "TargetedDecision" | "DecisionTimeout"
    ) {
        let result = (|| -> Result<(), String> {
            let power = content.power_content()?;
            let decision = power
                .decisions
                .decision(token.as_bytes())?
                .ok_or_else(|| format!("decision {token} is not defined in recorded content"))?;
            let name = (|| -> Result<Option<String>, String> {
                let own = own.ok_or_else(|| {
                    format!("decision {token}: country identity is unavailable at action hour")
                })?;
                let look = own.naming.look.as_ref().ok_or_else(|| {
                    format!("decision {token}: CountryAppearance is unavailable at action hour")
                })?;
                let group = own.ruling_group.as_ref().map_err(Clone::clone)?;
                let key = decision.name_key(
                    &own.tag,
                    &look.cosmetic_tag,
                    group.as_deref().unwrap_or_default(),
                    |key| content.defines(key),
                )?;
                let selects_state = decision.name_selects_state(|body| {
                    let frame = frame.ok_or_else(|| {
                        format!("decision {token} name selector: event-hour frame is unavailable")
                    })?;
                    texts
                        .world
                        .get_or_init(|| event_world(content, frame))
                        .as_ref()
                        .map_err(Clone::clone)?
                        .has_children(body)
                })?;
                localise_at(
                    content,
                    &key,
                    frame,
                    texts,
                    Some(own.naming.slot),
                    Some((row, selects_state)),
                    false,
                )
            })();
            match name {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            };
            let own_sprite = decision.sprite_candidates(None)?.remove(0);
            if content
                .catalogue
                .sprites
                .find(&own_sprite)
                .map_err(|why| why.to_string())?
                .is_some()
            {
                found.sprite = Some(own_sprite);
                return Ok(());
            }
            let selected = decision.select_icon(|body| {
                if body.is_empty() {
                    return Ok(true);
                }
                let frame = frame.ok_or_else(|| {
                    format!("decision {token} icon: event-hour frame is unavailable")
                })?;
                let own = own.ok_or_else(|| {
                    format!("decision {token} icon: country identity is unavailable at action hour")
                })?;
                let world = texts
                    .world
                    .get_or_init(|| event_world(content, frame))
                    .as_ref()
                    .map_err(Clone::clone)?;
                let owner = dashboard_text::triggers::Scope::Country(own.naming.slot);
                let mut scopes = dashboard_text::triggers::Scopes::of(owner);
                // Preserve the fresh self links.
                scopes.from = Some(owner);
                scopes.prev = Some(owner);
                // Until a caller's recorded links establish FROM/PREV, any trigger
                // that reads one refuses instead of guessing a target. Artwork FROM
                // is set to the recorded country or state target.
                if row.cause == "TargetedDecision"
                    || row.item.target_country.is_some()
                    || row.item.target_state.is_some()
                {
                    scopes.from = match (row.item.target_country, row.item.target_state) {
                        (Some(raw), _) => world
                            .slot(raw)
                            .map(dashboard_text::triggers::Scope::Country),
                        (None, Some(state)) => Some(dashboard_text::triggers::Scope::State(state)),
                        _ => None,
                    };
                    scopes.unknown.from = scopes.from.is_none();
                }
                world.evaluate(body, scopes).decided()
            })?;
            for sprite in decision.sprite_candidates(selected)?.into_iter().skip(1) {
                if content
                    .catalogue
                    .sprites
                    .find(&sprite)
                    .map_err(|why| why.to_string())?
                    .is_some()
                {
                    found.sprite = Some(sprite);
                    break;
                }
            }
            Ok(())
        })();
        if let Err(reason) = result {
            found.unknown = Some(format!("decision {token}: {reason}"));
        }
        return Some(found);
    }
    if row.cause == "Focus" {
        let result = (|| -> Result<(), String> {
            let power = content.power_content()?;
            let focus = power
                .focuses
                .get(token)
                .ok_or_else(|| format!("focus {token} is not defined in recorded content"))?;
            // The owner scope starts self-linked and receives a seed from campaign
            // and focus inputs. Those caller inputs are not recorded; only a
            // reached random read of this seed is unknown. Name and art stay independent.
            let name = localise_at(
                content,
                &focus.localisation,
                frame,
                texts,
                own.map(|owner| owner.naming.slot),
                None,
                true,
            );
            match name {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            };
            let art = (|| -> Result<Option<String>, String> {
                let own = own.ok_or_else(|| {
                    format!("focus {token} art: country identity unavailable at action hour")
                })?;
                // Try country-qualified then token-qualified overrides before
                // either normal or alternate icon selection.
                for sprite in focus.sprite_overrides(token, Some(&own.tag)) {
                    if content
                        .catalogue
                        .sprites
                        .find(&sprite)
                        .map_err(|why| why.to_string())?
                        .is_some()
                    {
                        return Ok(Some(sprite));
                    }
                }
                // The interface-window mode is not recorded.
                if focus.alternate_icon_present {
                    return Err(format!(
                        "focus {token} art: alternate icon mode is not recorded"
                    ));
                }
                // Evaluate the first null/true ScriptedKey case, including a singleton.
                // Its caller's fresh owner scope has FROM/PREV self.
                focus
                    .select_normal_icon(|body| {
                        if body.is_empty() {
                            return Ok(true);
                        }
                        let frame = frame.ok_or_else(|| {
                            format!("focus {token} art: event-hour frame is unavailable")
                        })?;
                        let world = texts
                            .world
                            .get_or_init(|| event_world(content, frame))
                            .as_ref()
                            .map_err(Clone::clone)?;
                        let owner = dashboard_text::triggers::Scope::Country(own.naming.slot);
                        let mut scopes = dashboard_text::triggers::Scopes::of(owner);
                        scopes.from = Some(owner);
                        scopes.prev = Some(owner);
                        world.evaluate(body, scopes).decided()
                    })
                    .map(|sprite| Some(sprite.to_owned()))
            })();
            match art {
                Ok(sprite) => found.sprite = sprite,
                Err(reason) => {
                    found.unknown = Some(match found.unknown.take() {
                        Some(previous) => format!("{previous}; {reason}"),
                        None => reason,
                    })
                }
            }
            Ok(())
        })();
        if let Err(reason) = result {
            found.unknown = Some(reason);
        }
        return Some(found);
    }
    if matches!(row.cause.as_str(), "Idea" | "Advisor" | "AdvisorRemoval") {
        let own = own?;
        let slot = Some(own.naming.slot);
        if row.cause == "Idea" {
            match content.idea_name(token, own.naming.for_tag(&own.tag)) {
                Ok(dashboard_text::idea_names::IdeaName::Key(key)) => {
                    match localise_at(content, &key, frame, texts, slot, None, false) {
                        Ok(name) => found.name = name,
                        Err(reason) => found.unknown = Some(reason),
                    }
                }
                Ok(dashboard_text::idea_names::IdeaName::Generated(name)) => {
                    found.name = Some(name)
                }
                Err(reason) => found.unknown = Some(reason),
            }
        }
        found.description = content.idea_description_key(token).and_then(|key| {
            match localise_at(content, &key, frame, texts, slot, None, false) {
                Ok(description) => description,
                Err(reason) => {
                    combine_reason(&mut found.unknown, Some(reason));
                    None
                }
            }
        });
        return Some(found);
    }
    None
}
/// A sprite's artwork, decoded once per reply however many rows show it.
fn sprite_png(
    content: &PoliticsContent,
    pngs: &mut BTreeMap<String, Result<Option<String>, String>>,
    name: &str,
) -> Result<Option<String>, String> {
    pngs.entry(name.to_owned())
        .or_insert_with(|| content.sprite_png(name).map_err(|error| error.to_string()))
        .clone()
}
fn character_art(
    content: &PoliticsContent,
    recorded: &facts::CharacterFact,
) -> (Option<String>, Option<String>, Option<String>) {
    let name = facts::character_name(recorded, content);
    match dashboard_base::products::character_portraits::portrait_png(content, &recorded.portrait) {
        Ok(icon) => (name, Some(icon), None),
        Err(reason) => (
            name,
            None,
            Some(format!("character {} portrait: {reason}", recorded.id)),
        ),
    }
}
#[cfg(test)]
thread_local! {
    static RESOLVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn resolve_item(
    scan: &Scanned,
    row: &LedgerMutation,
    facts: &EventFacts,
    content: Option<&PoliticsContent>,
    pngs: &mut BTreeMap<String, Result<Option<String>, String>>,
    id: String,
    daily_component: Option<&str>,
) -> (LedgerSpendItem, Option<IdeaInfo>, (String, String)) {
    #[cfg(test)]
    RESOLVED.with(|count| count.set(count.get() + 1));
    let own = scan.name_at(row.country, row.hour);
    let target = row
        .item
        .target_country
        .and_then(|raw| scan.name_at(raw, row.hour));
    let token = recorded_token(facts, row).map(str::to_owned);
    let recorded = facts.character.as_deref();
    let character = row
        .item
        .character
        .or_else(|| recorded.map(|recorded| recorded.id));
    let (category_id, category_name) = category(facts.native_cause);
    let mut category = (category_id.to_owned(), category_name.to_owned());
    let mut item = LedgerSpendItem {
        id,
        group_id: String::new(),
        name: None,
        action: String::new(),
        icon: None,
        tooltip_token: None,
        character,
        target_tag: target.map(|target| target.tag.clone()),
        target_name: target.and_then(|target| target.name.clone()),
        unknown: None,
    };
    let mut info = None;
    if let Some(content) = content {
        // Use topbar titles where defined. Broad laws/advisors have no
        // identified title and retain fixed cause words.
        let key = match category.0.as_str() {
            "diplomacy" => Some("TOPBAR_DIPLOMACY"),
            "decisions" => Some("TOPBAR_DECISIONVIEW"),
            _ => None,
        };
        if let Some(key) = key
            && let Some(name) = content.localised(key, View::Plain)
        {
            category.1 = name;
        }
        if row.cause == "Idea"
            && let Some(token) = token.as_deref()
            && let Ok(Some(idea)) = content.idea(token)
            && !idea.law
        {
            // Category names are plain; slots join in database order.
            if let Some(authored) = content
                .ideas
                .tags
                .categories()
                .0
                .iter()
                .find(|category| category.slots.iter().any(|slot| slot == &idea.group))
            {
                category.0 = format!("ideas:{}", authored.name);
                category.1 = content
                    .localised(&authored.name, View::Plain)
                    .unwrap_or_else(|| "Not localised".into());
            }
        }
        if matches!(row.cause.as_str(), "Idea" | "Advisor" | "AdvisorRemoval") {
            if let Some(token) = token.as_deref() {
                if content
                    .idea(token)
                    .ok()
                    .flatten()
                    .is_some_and(|idea| idea.law)
                {
                    category = ("laws".into(), "Laws".into());
                }
                if let Some(own) = own {
                    let script = recorded.and_then(|recorded| recorded.script_token.as_deref());
                    let (art, problem) =
                        content.idea_artwork(token, script, own.naming.for_tag(&own.tag));
                    item.name = art.name;
                    item.icon = art.icon;
                    item.unknown = problem;
                    if let Some(recorded) = recorded {
                        let (name, icon, reason) = character_art(content, recorded);
                        item.name = name;
                        if icon.is_some() {
                            item.icon = icon;
                            // The recorded portrait supersedes missing fallback
                            // art; an independently refused definition remains.
                            if content.idea(token).is_ok() && content.advisor(token).is_ok() {
                                item.unknown = None;
                            }
                        }
                        item.unknown = reason.or(item.unknown);
                    }
                    let built = recorded.map(|recorded| &recorded.template);
                    let localise = |key: &str, view: View| content.localised(key, view);
                    let joined = idea_history::Joined::of(content, &localise);
                    let (found, _) = idea_history::prepare_info(
                        &joined,
                        token,
                        std::str::from_utf8(&row.item.slot_key)
                            .ok()
                            .filter(|slot| !slot.is_empty()),
                        built.map(|built| built.as_ref().map_err(String::as_str)).unwrap_or_else(|| Err("the recorded holder/template is unavailable at the action hour")),
                        false,
                    );
                    info = Some(found);
                } else {
                    item.unknown = Some(format!(
                        "{token}: country identity unavailable at action hour"
                    ));
                }
            }
        } else if let Some(token) = token.as_deref() {
            // Abilities and traits display their recorded token's localisation.
            // Refuse scripted text without its event scope rather than print it.
            item.name = None;
            if matches!(
                row.cause.as_str(),
                "Decision" | "TargetedDecision" | "DecisionTimeout" | "Focus" | "AbilityActivation"
            ) {
                match content.power_content() {
                    Err(reason) => item.unknown = Some(reason),
                    Ok(power) => match row.cause.as_str() {
                        "Decision" | "TargetedDecision" | "DecisionTimeout" => {} // event_text owns native event-scope resolution.
                        "Focus" => {} // event_text owns the native dated text and icon identity.
                        "AbilityActivation" => {
                            if let Some(key) = power.abilities.name_key(token.as_bytes()) {
                                item.name = content.localised(key, View::Plain);
                            }
                            if let Some(sprite) = power.abilities.static_sprite(token.as_bytes()) {
                                match sprite_png(content, pngs, sprite) {
                                    Ok(icon) => item.icon = icon,
                                    Err(reason) => {
                                        item.unknown = Some(format!(
                                            "ability {token} sprite {sprite}: {reason}"
                                        ))
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
        }
        if matches!(
            row.cause.as_str(),
            "LeaderPromotion"
                | "LeaderAssignment"
                | "LeaderRecruitment"
                | "ScientistRecruitment"
                | "ScientistFaction"
                | "OfficerUpgrade"
                | "CaptainUpgrade"
        ) && let Some(id) = character
        {
            if let Some(recorded) = recorded {
                let (name, icon, reason) = character_art(content, recorded);
                item.name = name;
                item.icon = icon;
                item.unknown = reason;
            } else {
                item.unknown = Some(format!(
                    "character {id}: unit-leader portrait role unavailable at action hour"
                ));
            }
        }
    } else {
        item.unknown = Some("recorded content has not been resolved".into());
    }
    if let Some(content) = content {
        if let Some(text) = facts.text.as_deref() {
            if !matches!(row.cause.as_str(), "Advisor" | "AdvisorRemoval") {
                item.name = text.name.clone();
            }
            combine_reason(&mut item.unknown, text.unknown.clone());
            if let Some(sprite) = text.sprite.as_deref() {
                match sprite_png(content, pngs, sprite) {
                    Ok(icon) => item.icon = icon,
                    Err(reason) => item.unknown = Some(format!("sprite {sprite}: {reason}")),
                }
            }
        }
        if row.cause == "Diplomacy" {
            // The daily relation title uses the recorded relation TYPE_ID token,
            // not a diplomatic action.
            let key = if daily_component == Some("Relation") {
                Some(installed_content::diplomacy::improve_relation_title_key().to_owned())
            } else if !row.item.key.is_empty() {
                installed_content::diplomacy::scripted_action_name_key(
                    std::str::from_utf8(&row.item.key).unwrap_or_default(),
                )
                .ok()
            } else if row
                .amount_raw
                .as_deref()
                .and_then(|raw| raw.parse::<i128>().ok())
                .is_some_and(|raw| raw < 0)
            {
                row.item
                    .token
                    .and_then(installed_content::diplomacy::charged_action_name_key)
                    .map(str::to_owned)
            } else {
                None
            };
            item.name = key
                .as_deref()
                .and_then(|key| content.localised(key, View::Plain));
            if key.is_none() {
                item.unknown = Some(format!(
                    "diplomatic action token {:?}: native action-name branch is not recorded/identified",
                    row.item.token
                ));
            }
        }
        if row.cause == "LeaderTrait"
            && let Some(token) = token.as_deref()
        {
            match content.power_content() {
                Err(reason) => item.unknown = Some(reason),
                Ok(power) => {
                    if let Some(own) = own {
                        match power
                            .unit_traits
                            .name_key(token, &own.tag, |key| content.defines(key))
                        {
                            Ok(key) => {
                                item.name = key.and_then(|key| content.localised(&key, View::Plain))
                            }
                            Err(reason) => item.unknown = Some(reason.into()),
                        }
                    } else {
                        item.unknown = Some(format!(
                            "trait {token}: country identity unavailable at action hour"
                        ));
                    }
                    match power.unit_traits.assignment_sprite(token) {
                        Ok(Some(sprite)) => match sprite_png(content, pngs, &sprite) {
                            Ok(icon) => item.icon = icon,
                            Err(reason) => {
                                item.unknown =
                                    Some(format!("trait {token} sprite {sprite}: {reason}"))
                            }
                        },
                        Ok(None) => {
                            item.unknown = Some(format!(
                                "trait {token} is not registered in recorded content"
                            ))
                        }
                        Err(reason) => item.unknown = Some(reason.into()),
                    }
                }
            }
        }
    }
    if row.cause == "DailyGain"
        && let Some(content) = content
    {
        match daily_component {
            Some("PoliticalPowerCostModifier") => {
                if let Some(modifier) = facts.modifier.as_deref() {
                    let key = std::str::from_utf8(&modifier.name_key).ok();
                    let argument = std::str::from_utf8(&modifier.argument).ok();
                    match key.zip(argument) {
                        Some((key, argument))
                            if content.catalogue.localisation.get(key).is_some() =>
                        {
                            // Modifier names substitute the recorded argument as ARG (type 1).
                            let arguments =
                                [("ARG", installed_content::localisation::Argument::Text(argument))];
                            let arguments: &[_] =
                                if argument.is_empty() { &[] } else { &arguments };
                            match dashboard_text::catalogue::localised_with(
                                &content.catalogue.localisation,
                                key,
                                arguments,
                                View::Scripted,
                            ) {
                                Ok(name) => item.name = Some(name),
                                Err(reason) => item.unknown = Some(reason),
                            }
                        }
                        Some((key, _)) => {
                            item.unknown = Some(format!("daily cost modifier {key}: not localised"))
                        }
                        None => {
                            item.unknown =
                                Some("daily cost modifier definition text is not UTF-8".into())
                        }
                    }
                } else {
                    item.unknown = Some("daily cost modifier index 0x26: ModifierDefinitions is unavailable at action hour".into());
                }
            }
            Some("MioResearch" | "MioProduction") => {
                if let Some(organisation) = facts.organisation.as_deref() {
                    // The scoped name is frozen by event_text at this read. Only artwork
                    // is loaded at presentation. Reuse the production kit's recorded
                    // owner/key precedence and icon fallback.
                    let sprite =
                        dashboard_base::products::production_lines::organisation_icon_with_token(
                            organisation,
                            facts.organisation_token_known,
                            |sprite| {
                                content
                                    .catalogue
                                    .sprites
                                    .find(sprite)
                                    .ok()
                                    .flatten()
                                    .is_some()
                            },
                        );
                    match sprite {
                        Ok(Some(sprite)) => match sprite_png(content, pngs, &sprite) {
                            Ok(icon) => item.icon = icon,
                            Err(reason) => {
                                item.unknown = Some(format!(
                                    "organisation {} sprite {sprite}: {reason}",
                                    organisation.token
                                ))
                            }
                        },
                        Ok(None) => {}
                        Err(reason) => combine_reason(
                            &mut item.unknown,
                            Some(format!(
                                "organisation {:?}: {reason}",
                                row.item.object_reference
                            )),
                        ),
                    }
                    if item.name.is_none() && item.unknown.is_none() {
                        item.unknown = Some(format!(
                            "organisation {}: recorded name is not localised or needs unavailable scope",
                            organisation.token
                        ));
                    }
                } else {
                    item.unknown = Some(format!(
                        "organisation {:?}: IndustrialOrganisations identity is unavailable at action hour",
                        row.item.object_reference
                    ));
                }
            }
            _ => {}
        }
    }
    // The net daily CP gain can be a negative operand: that is an
    // identified deficit, not an unidentified authored purchase.
    if row.currency == "Command" && row.cause == "DailyGain" {
        item.name = Some("Daily deficit".into());
        item.unknown = None;
    }
    if item.name.is_none()
        && !matches!(row.cause.as_str(), "NotIdentified" | "AbilityUpkeep")
        && item.unknown.is_none()
    {
        item.unknown = Some(format!("{} item {:?} is not identified", row.cause, token));
    }
    let name = item.name.as_deref().unwrap_or("Not identified");
    let target = item.target_name.as_deref().unwrap_or("Not identified");
    item.action = match row.cause.as_str() {
        "Advisor" => format!(
            "{} {name}",
            if row.item.action == "Replace" {
                "Replaced advisor with"
            } else {
                "Hired"
            }
        ),
        "AdvisorRemoval" => format!("Removed advisor: {name}"),
        "Idea" if category.0 == "laws" => name.to_owned(),
        "Idea" => format!(
            "{} {name}",
            match row.item.action.as_str() {
                "Remove" => "Removed",
                "Replace" => "Replaced with",
                _ => "Changed to",
            }
        ),
        "Decision" | "TargetedDecision" | "DecisionTimeout" => format!(
            "{name}{}",
            if row.item.target_country.is_some() {
                format!(": {target}")
            } else {
                String::new()
            }
        ),
        "Diplomacy" => format!(
            "{}: {target}",
            item.name.as_deref().unwrap_or("Not identified")
        ),
        "WarJustification" => format!("War justification: {target}"),
        "JoinWar" => format!("Joined war: {target}"),
        "Focus" => format!("Started focus: {name}"),
        "AbilityActivation" => format!("Activated: {name}"),
        "AbilityUpkeep" => "Ability upkeep".into(),
        "DailyGain" if row.currency == "Command" => "Daily deficit".into(),
        "LeaderTrait" => format!("Learned trait: {name}"),
        "LeaderPromotion" => format!("Promoted: {name}"),
        "LeaderAssignment" => format!("Assigned: {name}"),
        "LeaderRecruitment" => format!("Recruited: {name}"),
        "NotIdentified" => "Not identified".into(),
        _ => format!("{}: {name}", category.1),
    };
    let template = recorded
        .and_then(|recorded| recorded.template.as_ref().ok())
        .map(|template| {
            (
                &template.file,
                template.line,
                &template.tag,
                template.generated,
            )
        });
    let mut identity = row.item.clone();
    identity.action.clear();
    item.group_id = serde_json::to_string(&(
        &category.0,
        &identity,
        &token,
        &character,
        &template,
        &item.target_tag,
        &item.target_name,
        &item.name,
    ))
    .expect("display identity serializes");
    // Equal dated identity (including resolved name, description and portrait)
    // shares one tooltip and one artwork payload across repeated mutations.
    if let Some(found) = info.as_mut() {
        let tooltip = serde_json::to_string(&(
            &item.group_id,
            &item.name,
            &own.map(|own| format!("{:?}", own.naming)),
            &recorded.map(|recorded| format!("{:?}", recorded.portrait)),
            &found,
            facts
                .text
                .as_ref()
                .and_then(|text| text.description.as_deref()),
        ))
        .expect("dated tooltip identity serializes");
        found.token = tooltip.clone();
        item.tooltip_token = Some(tooltip);
    }
    item.unknown = item.unknown.map(|reason| undated_reason(&reason));
    (item, info, category)
}
fn sum_raw(values: impl Iterator<Item = Option<i128>>) -> io::Result<Option<i128>> {
    let mut total = 0i128;
    for value in values {
        let Some(value) = value else {
            return Ok(None);
        };
        total = total.checked_add(value).ok_or_else(|| {
            io::Error::other("Politics spending sum exceeds fixed-point integer range")
        })?;
    }
    Ok(Some(total))
}
struct ReplyBudget {
    bytes: usize,
}
impl ReplyBudget {
    fn new() -> Self {
        Self { bytes: 4096 }
    }
    fn add<T: Serialize + ?Sized>(&mut self, value: &T) -> io::Result<()> {
        let room = MAX_REPLY_BYTES.saturating_sub(self.bytes);
        let bytes = json_len(&value, room).ok_or_else(|| {
            io::Error::other(
                "Politics ledger exceeds the complete-response budget; no history was truncated",
            )
        })?;
        self.bytes = self
            .bytes
            .checked_add(bytes + 2)
            .ok_or_else(|| io::Error::other("Politics response size overflow"))?;
        if self.bytes > MAX_REPLY_BYTES {
            return Err(io::Error::other(
                "Politics ledger exceeds the complete-response budget; no history was truncated",
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
pub(super) fn present(
    scan: &Scanned,
    branch: u64,
    tag: &str,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
) -> io::Result<PoliticsLedgerReply> {
    present_view(scan, branch, tag, content, cancelled, LedgerView::Raw)
}
/// GovernmentChanges retains the old group before destruction.
/// Late publication cannot borrow today's identities.
fn government_token(
    frame: &DecodedFrame,
    previous: Option<&DecodedFrame>,
    hour: i32,
    token: Option<u32>,
) -> Option<String> {
    let event_frame = if frame.hour == hour {
        Some(frame)
    } else {
        previous.filter(|previous| previous.hour == hour)
    }?;
    facts::lexical_token(event_frame, token?)
}

#[cfg(any(test, feature = "test-support"))]
fn present_view(
    scan: &Scanned,
    branch: u64,
    tag: &str,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
    view: LedgerView,
) -> io::Result<PoliticsLedgerReply> {
    let balance = view == LedgerView::Balance;
    present_fields(
        scan, branch, tag, content, cancelled, view, balance, true, None, None,
    )
}

#[derive(Default, Serialize, Deserialize)]
struct PresentationNotes {
    facts: BTreeMap<String, BTreeMap<String, u64>>,
    formats: BTreeSet<String>,
}
fn presented_reason(
    reasons: &mut BTreeMap<String, u64>,
    notes: &mut Option<&mut PresentationNotes>,
    id: &str,
    reason: String,
) -> io::Result<()> {
    if let Some(notes) = notes.as_deref_mut() {
        count_reason(notes.facts.entry(id.into()).or_default(), reason.clone(), 1)?;
    }
    count_reason(reasons, reason, 1)
}

#[allow(clippy::too_many_arguments)]
fn present_fields(
    scan: &Scanned,
    branch: u64,
    tag: &str,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
    view: LedgerView,
    balance: bool,
    intern: bool,
    mut tooltip_order: Option<&mut Vec<(String, bool)>>,
    mut notes: Option<&mut PresentationNotes>,
) -> io::Result<PoliticsLedgerReply> {
    let mut budget = ReplyBudget::new();
    let empty = PoliticsLedgerReply {
        branch_id: branch.to_string(),
        tag: tag.into(),
        decimals: 5,
        ..Default::default()
    };
    let unrecorded = Country::default();
    let raw = scan.countries.get(tag).unwrap_or(&unrecorded);
    let mut reply = if view == LedgerView::Governments {
        empty.clone()
    } else if view != LedgerView::Raw {
        budget.add(&raw.spending_coverage)?;
        let daily: Vec<_> = if view == LedgerView::Balance {
            raw.daily.iter().map(Daily::ledger).collect()
        } else {
            Vec::new()
        };
        if view == LedgerView::Balance {
            budget.add(&daily)?;
        }
        PoliticsLedgerReply {
            spending_coverage: raw.spending_coverage.clone(),
            daily,
            ..empty.clone()
        }
    } else {
        let reply = PoliticsLedgerReply {
            mutations: raw.mutations.iter().map(Mutation::ledger).collect(),
            governments: raw.governments.clone(),
            daily: raw.daily.iter().map(Daily::ledger).collect(),
            daily_costs: raw.daily_costs.iter().map(DailyCost::ledger).collect(),
            coverage: raw.coverage.clone(),
            spending_coverage: raw.spending_coverage.clone(),
            ..empty.clone()
        };
        budget.add(&reply)?;
        reply
    };
    // Ruling party shares the same all-country scan without requesting payments,
    // daily rates, holds or their artwork merely to name execution provenance.
    if view == LedgerView::Governments {
        cancelled_request(cancelled)?;
        budget.add(&raw.governments)?;
        reply.governments = raw.governments.clone();
        for key in [None, Some(tag.to_owned())] {
            if let Some(notes) = scan.warnings.get(&key) {
                for (reason, count) in notes {
                    let warning = format!("{reason} (count: {count})");
                    budget.add(&warning)?;
                    reply.warnings.push(warning);
                }
            }
        }
        if json_len(&reply, MAX_REPLY_BYTES).is_none() {
            return Err(io::Error::other(
                "Government history exceeds the complete-response budget; no history was truncated",
            ));
        }
        return Ok(reply);
    }
    let mut reasons = BTreeMap::new();
    for key in [None, Some(tag.to_owned())] {
        if let Some(notes) = scan.warnings.get(&key) {
            for (reason, count) in notes {
                count_reason(&mut reasons, reason.clone(), *count)?;
            }
        }
    }
    let mut tooltip_ids = BTreeMap::new();
    let mut infos = Vec::new();
    let mut info_ids = BTreeSet::new();
    let mut art = Vec::new();
    let mut categories = BTreeMap::new();
    let mut items = BTreeMap::new();
    let mut item_ids = BTreeSet::new();
    let mut upkeep = BTreeMap::new();
    let mut pngs = BTreeMap::new();
    // The balance sentence uses the spending resolution before tooltip IDs,
    // icons and grouped upkeep change its presentation.
    let mut balance_items = Vec::with_capacity(if balance { raw.mutations.len() } else { 0 });
    for stored in &raw.mutations {
        if balance {
            balance_items.push(None);
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Politics request cancelled",
            ));
        }
        let row = &stored.ledger();
        let Some(charge) = charge(row) else {
            continue;
        };
        let id = format!(
            "{}:{}:{}:{}",
            row.country, row.instance, row.epoch, row.sequence
        );
        let (mut item, mut info, category) = resolve_item(
            scan,
            row,
            &stored.facts,
            content,
            &mut pngs,
            id.clone(),
            None,
        );
        if balance {
            *balance_items.last_mut().unwrap() = Some((item.action.clone(), item.unknown.clone()));
        }
        if intern {
            intern_tooltip(&mut item, &mut info, &mut tooltip_ids, tag);
        } else if let Some(order) = tooltip_order.as_mut() {
            order.extend(item.tooltip_token.clone().map(|token| (token, false)));
        }
        if let Some(reason) = &item.unknown {
            presented_reason(&mut reasons, &mut notes, &id, reason.clone())?;
        }
        if let Some(info) = info
            && info_ids.insert(info.token.clone())
        {
            let artwork = dashboard_core::dto::Artwork {
                token: info.token.clone(),
                name: item.name.clone(),
                description: stored
                    .facts
                    .text
                    .as_ref()
                    .and_then(|text| text.description.clone()),
                icon: item.icon.clone(),
                base_duration_days: None,
            };
            budget.add(&artwork)?;
            budget.add(&info)?;
            art.push(artwork);
            infos.push(info);
        }
        if item.tooltip_token.is_some() {
            item.icon = None;
        }
        categories.insert(category.0.to_owned(), category.1.clone());
        let change = LedgerSpendChange {
            hour: Some(row.hour),
            read_hour: row.read_hour,
            charge_raw: charge.map(|raw| raw.to_string()),
            realized_delta_raw: Some(row.realized_delta_raw.clone()),
            occurrences: 1,
            covered: row.covered,
        };
        if row.currency == "Command" && row.cause == "AbilityUpkeep" {
            let key = (
                row.country,
                row.instance,
                row.epoch.clone(),
                row.hour.div_euclid(24),
            );
            if let Some(&index) = upkeep.get(&key) {
                budget.add(&change)?;
                let spend: &mut LedgerSpend = &mut reply.spending[index];
                spend.changes.push(change);
                spend.covered &= row.covered;
                spend.occurrences += 1;
                continue;
            }
            upkeep.insert(key, reply.spending.len());
            item.name = None;
            item.icon = None;
            item.tooltip_token = None;
            item.character = None;
            item.unknown = None;
            item.action = "Ability upkeep".into();
        }
        let item_id = serde_json::to_string(&(
            &item.group_id,
            &item.tooltip_token,
            &item.action,
            &item.icon,
            &item.unknown,
        ))
        .map_err(io::Error::other)?;
        let item_id = items
            .entry(item_id)
            .or_insert_with(|| item.id.clone())
            .clone();
        item.id = item_id.clone();
        if item_ids.insert(item_id.clone()) {
            budget.add(&item)?;
            reply.spend_items.push(item);
        }
        let spend = LedgerSpend {
            country_id: row.country,
            id,
            sequence: row.sequence.clone(),
            ordinal: 0,
            hour: row.hour,
            read_hour: row.read_hour,
            country_tag: tag.into(),
            country_name: scan
                .name_at(row.country, row.hour)
                .and_then(|name| name.name.clone()),
            currency: row.currency.clone(),
            category: category.0,
            item_id,
            charge_raw: charge.map(|raw| raw.to_string()),
            realized_delta_raw: Some(row.realized_delta_raw.clone()),
            covered: row.covered,
            occurrences: 1,
            changes: vec![change],
        };
        budget.add(&spend)?;
        reply.spending.push(spend);
    }
    // Daily-rate components and later TimedActivity subtractions are
    // different applications. Never count the net DailyGain mutation again,
    // nor apportion its realized/clamped delta.
    let daily_applications: BTreeSet<_> = raw
        .daily
        .iter()
        .filter_map(|day| {
            let day = &day.row;
            day.pp_sequence
                .map(|sequence| (day.country, day.instance, day.epoch, day.day_hour, sequence))
        })
        .collect();
    for stored in &raw.daily_costs {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Politics request cancelled",
            ));
        }
        let recorded = &stored.row;
        let id = format!(
            "daily:{}:{}:{}:{}:{}",
            recorded.country,
            recorded.instance,
            recorded.epoch,
            recorded.day_hour,
            recorded.ordinal
        );
        if let Some(notes) = notes.as_deref_mut() {
            notes.facts.entry(id.clone()).or_default();
        }
        let amount = i128::from(recorded.cost_raw);
        if amount == 0 {
            continue;
        }
        if !daily_applications.contains(&(
            recorded.country,
            recorded.instance,
            recorded.epoch,
            recorded.day_hour,
            recorded.pp_sequence,
        )) {
            invalidate_read(
                &mut reply.spending_coverage,
                stored.read_hour,
                "Daily political power costs have no matched payment at this read",
            );
            presented_reason(
                &mut reasons,
                &mut notes,
                &id,
                "Daily political power cost: matched PP application is unavailable".into(),
            )?;
            continue;
        }
        let cost = &stored.ledger();
        let id = format!(
            "daily:{}:{}:{}:{}:{}",
            cost.country, cost.instance, cost.epoch, cost.day_hour, cost.ordinal
        );
        let row = daily_mutation(cost);
        let (mut item, mut info, mut category) = resolve_item(
            scan,
            &row,
            &stored.facts,
            content,
            &mut pngs,
            id.clone(),
            Some(&cost.component),
        );
        if amount < 0 {
            let name = item.name.as_deref().unwrap_or("Not identified");
            presented_reason(
                &mut reasons,
                &mut notes,
                &id,
                format!(
                    "Negative daily political power cost component {name}: recorded credit is not included in spending"
                ),
            )?;
            invalidate_read(
                &mut reply.spending_coverage,
                cost.read_hour,
                "Negative daily political power cost components are not included in spending",
            );
            continue;
        }
        if intern {
            intern_tooltip(&mut item, &mut info, &mut tooltip_ids, tag);
        } else if let Some(order) = tooltip_order.as_mut() {
            order.extend(item.tooltip_token.clone().map(|token| (token, true)));
        }
        if let Some(reason) = &item.unknown {
            presented_reason(&mut reasons, &mut notes, &id, reason.clone())?;
        }
        if cost.cause == "DailyGain" {
            category = match cost.component.as_str() {
                "MioResearch" | "MioProduction" => {
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
        if let Some(info) = info
            && info_ids.insert(info.token.clone())
        {
            let artwork = dashboard_core::dto::Artwork {
                token: info.token.clone(),
                name: item.name.clone(),
                description: stored
                    .facts
                    .text
                    .as_ref()
                    .and_then(|text| text.description.clone()),
                icon: item.icon.clone(),
                base_duration_days: None,
            };
            budget.add(&artwork)?;
            budget.add(&info)?;
            art.push(artwork);
            infos.push(info);
        }
        if item.tooltip_token.is_some() {
            item.icon = None;
        }
        categories.insert(category.0.clone(), category.1.clone());
        let item_id = serde_json::to_string(&(
            &item.group_id,
            &item.tooltip_token,
            &item.action,
            &item.icon,
            &item.unknown,
        ))
        .map_err(io::Error::other)?;
        let item_id = items
            .entry(item_id)
            .or_insert_with(|| item.id.clone())
            .clone();
        item.id = item_id.clone();
        if item_ids.insert(item_id.clone()) {
            budget.add(&item)?;
            reply.spend_items.push(item);
        }
        let spend = LedgerSpend {
            country_id: cost.country,
            id,
            sequence: cost.pp_sequence.clone(),
            ordinal: cost.ordinal,
            hour: cost.day_hour,
            read_hour: cost.read_hour,
            country_tag: tag.into(),
            country_name: scan
                .name_at(cost.country, cost.day_hour)
                .and_then(|name| name.name.clone()),
            currency: "Political".into(),
            category: category.0,
            item_id,
            charge_raw: Some(amount.to_string()),
            realized_delta_raw: None,
            covered: cost.covered,
            occurrences: 1,
            changes: vec![LedgerSpendChange {
                hour: Some(cost.day_hour),
                read_hour: cost.read_hour,
                charge_raw: Some(amount.to_string()),
                realized_delta_raw: None,
                occurrences: 1,
                covered: cost.covered,
            }],
        };
        budget.add(&spend)?;
        reply.spending.push(spend);
    }
    for spend in &mut reply.spending {
        spend.changes.sort_by_key(|change| change.read_hour);
        spend.charge_raw = sum_raw(spend.changes.iter().map(|change| {
            change
                .charge_raw
                .as_deref()
                .and_then(|raw| raw.parse::<i128>().ok())
        }))?
        .map(|raw| raw.to_string());
        spend.realized_delta_raw = sum_raw(spend.changes.iter().map(|change| {
            change
                .realized_delta_raw
                .as_deref()
                .and_then(|raw| raw.parse::<i128>().ok())
        }))?
        .map(|raw| raw.to_string());
    }
    reply.spend_categories = categories
        .into_iter()
        .map(|(id, name)| LedgerSpendCategory {
            identified: id != "notIdentified",
            id,
            name,
        })
        .collect();
    reply.spending.sort_by(|a, b| {
        a.hour
            .cmp(&b.hour)
            .then_with(|| {
                a.sequence
                    .parse::<u64>()
                    .ok()
                    .cmp(&b.sequence.parse::<u64>().ok())
            })
            .then_with(|| a.ordinal.cmp(&b.ordinal))
            .then_with(|| a.id.cmp(&b.id))
    });
    // Fill the complement of actually supplied intervals; a lost country
    // table/block supplies no proof and can never become a zero total.
    let mut cursor = scan.first_hour;
    let supplied = std::mem::take(&mut reply.spending_coverage);
    for witness in supplied {
        let (Some(first), Some(last)) = (witness.since_hour, witness.through_hour) else {
            continue;
        };
        if let Some(cursor) = cursor
            && cursor < first
        {
            fill_gap(scan, tag, cursor, first - 1, &mut reply.spending_coverage);
        }
        reply.spending_coverage.push(witness);
        cursor = Some(last.saturating_add(1));
    }
    if let (Some(first), Some(last)) = (cursor, scan.last_hour)
        && first <= last
    {
        fill_gap(scan, tag, first, last, &mut reply.spending_coverage);
    }
    if let Some(content) = content {
        let (modifier_keys, text_keys) = idea_history::effect_keys(&infos);
        let (modifiers, warnings) = idea_format::modifier_formats_from_recorded(
            scan.modifier_definitions.as_ref(),
            &modifier_keys,
            content,
        );
        for warning in warnings {
            if let Some(notes) = notes.as_deref_mut() {
                notes.formats.insert(warning.clone());
            }
            count_reason(&mut reasons, warning, 1)?;
        }
        for modifier in &modifiers {
            budget.add(modifier)?;
        }
        let texts = idea_format::texts(content, &text_keys);
        for text in &texts {
            budget.add(text)?;
        }
        reply.idea_tooltips = Some(LedgerIdeaTooltips {
            ideas: infos,
            artwork: art,
            modifiers,
            texts,
        });
    }
    let holds = holds::history(scan, tag);
    budget.add(holds)?;
    reply.command_holds = holds.to_vec();
    if balance {
        for (stored, resolved) in raw.mutations.iter().zip(balance_items) {
            cancelled_request(cancelled)?;
            let row = &stored.ledger();
            let (action, reason) = match row.cause.as_str() {
                "DailyGain" => (
                    if row.currency == "Command"
                        && row
                            .amount_raw
                            .as_deref()
                            .and_then(|raw| raw.parse::<i128>().ok())
                            .is_some_and(|raw| raw < 0)
                    {
                        "Daily deficit"
                    } else {
                        "Daily income applied"
                    }
                    .to_owned(),
                    None,
                ),
                "CapCorrection" => ("Maximum correction".into(), None),
                "ResetSetup" => ("Balance set/reset".into(), None),
                "Console" => ("Console adjustment".into(), None),
                _ => match resolved {
                    Some(resolved) => resolved,
                    None => {
                        let (item, _, _) = resolve_item(
                            scan,
                            row,
                            &stored.facts,
                            content,
                            &mut pngs,
                            mutation_id(row),
                            None,
                        );
                        (item.action, item.unknown)
                    }
                },
            };
            if let Some(reason) = &reason
                && charge(row).is_none()
            {
                presented_reason(&mut reasons, &mut notes, &mutation_id(row), reason.clone())?;
            }
            let change = LedgerBalanceChange {
                country_id: row.country,
                hour: row.hour,
                read_hour: row.read_hour,
                currency: row.currency.clone(),
                amount_raw: row.amount_raw.clone(),
                realized_delta_raw: row.realized_delta_raw.clone(),
                action,
                covered: row.covered,
                reason,
            };
            budget.add(&change)?;
            reply.balance_changes.push(change);
        }
    }
    if let Some(content) = content {
        for warning in content.power_warnings() {
            if let Some(notes) = notes.as_deref_mut() {
                notes.formats.insert(warning.clone());
            }
            count_reason(&mut reasons, warning.clone(), 1)?;
        }
    }
    for (reason, count) in reasons {
        let warning = format!("{reason} (count: {count})");
        budget.add(&warning)?;
        reply.warnings.push(warning);
    }
    // The service separately measures its full envelope. Reserve envelope
    // room here so a product reply cannot allocate another over-limit JSON.
    if json_len(&reply, MAX_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "Politics ledger exceeds the complete-response budget; no history was truncated",
        ));
    }
    Ok(reply)
}

#[cfg(test)]
mod presentation_content_tests;
#[cfg(test)]
mod tests;
