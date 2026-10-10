//! A shared projection of payments. Signed PP operands are applied then
//! clamped; CP subtracts its operand. Upkeep commits on each paid activation
//! anniversary. Reconciliation, grants and refunds are never payments.
use super::*;
use crate::products::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
use crate::products::{
    idea_format::{self, LocalisedText, ModifierFormat},
    idea_history::{self, IdeaInfo},
    validate_tag,
};
use crate::products::{
    identity::{Look, NamingCountry, character_script_token},
    politics_content::{PoliticsContent, politics_content},
};
use crate::{
    catalogue::{Catalogue, View, plain_text},
    transport::json_len,
};
use installed_content::country_names::CountryNames;
use std::sync::Arc;
mod facts;
use facts::{EventFacts, store_facts};
mod presentation;
use presentation::{count_reason, fill_gap, intern_tooltip, invalidate_read};
mod holds;
pub use holds::{LedgerCommandHold, LedgerCommandHolder};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerIdeaTooltips {
    pub ideas: Vec<IdeaInfo>,
    pub artwork: Vec<crate::Artwork>,
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
struct Cache {
    branch: Option<u64>,
    catalogue: Option<Arc<Catalogue>>,
    scan: Option<Arc<Scanned>>,
}
#[derive(Default)]
pub(super) struct Scanned {
    countries: BTreeMap<String, PoliticsLedgerReply>,
    identities: BTreeMap<(u32, u32, u64), BTreeMap<i32, String>>,
    events: BTreeSet<(u32, u32, u64, u64)>,
    daily: BTreeMap<(u32, u32, u64, i32), usize>,
    costs: BTreeSet<(u32, u32, u64, i32, u32)>,
    naming: BTreeMap<(u32, i32), NamedCountry>,
    event_facts: BTreeMap<String, EventFacts>,
    modifier_definitions: Option<BTreeMap<String, ModifierDefinitionRow>>,
    modifier_views: Option<(replay::frame::ViewIdentity, replay::frame::ViewIdentity)>,
    event_text: BTreeMap<String, ItemText>,
    previous: Option<DecodedFrame>,
    warnings: BTreeMap<Option<String>, BTreeMap<String, u64>>,
    presence: Vec<PresenceRun>,
    known_tags: BTreeMap<u32, String>,
    refused: BTreeMap<u32, (String, u64)>,
    holds: holds::Holds,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct NamedCountry {
    tag: String,
    name: Option<String>,
    naming: NamingCountry,
    ruling_group: Result<Option<String>, String>,
}
#[derive(Default)]
struct ItemText {
    name: Option<String>,
    sprite: Option<String>,
    description: Option<String>,
    unknown: Option<String>,
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
    fn note(&mut self, tag: Option<&str>, note: String, count: u64) -> io::Result<()> {
        let key = tag.map(str::to_owned);
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
    fn country(&mut self, tag: &str, branch: u64) -> &mut PoliticsLedgerReply {
        self.countries
            .entry(tag.to_owned())
            .or_insert_with(|| PoliticsLedgerReply {
                branch_id: branch.to_string(),
                tag: tag.to_owned(),
                decimals: 5,
                ..Default::default()
            })
    }
    fn name_at(&self, raw: u32, hour: i32) -> Option<&NamedCountry> {
        self.naming.get(&(raw, hour))
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
    ) {
        if requested
            .iter()
            .all(|raw| self.naming.contains_key(&(*raw, frame.hour)))
        {
            return;
        }
        let names =
            content.map(|content| CountryNames::read(frame, &content.catalogue.localisation));
        let links = frame.complete::<tables::CountryLinks>().ok();
        let countries: BTreeMap<_, _> = frame
            .complete::<tables::Countries>()
            .unwrap_or_default()
            .iter()
            .map(|country| (country.canonical_slot, country))
            .collect();
        for raw in requested {
            if self.naming.contains_key(&(*raw, frame.hour)) {
                continue;
            }
            let slot = links.and_then(|links| crate::metadata::link_slot(links, *raw));
            let Some(country) = slot.and_then(|slot| countries.get(&slot).copied()) else {
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
        && old.through_hour.and_then(|hour| hour.checked_add(1)) == value.through_hour
    {
        old.through_hour = value.through_hour;
    } else {
        rows.push(value);
    }
}
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
    let Ok(countries) = frame.complete::<tables::Countries>() else {
        return Ok(());
    };
    let mut tags = BTreeSet::new();
    let mut refused_tags = BTreeSet::new();
    for country in countries {
        match valid_recorded_tag(country) {
            Ok(tag) => {
                tags.insert(tag.to_owned());
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
                    scan.note(Some(&tag), reason, 1)?;
                }
            }
        }
    }
    if let Some(last) = scan.presence.last_mut()
        && last.last.checked_add(1) == Some(frame.hour)
        && last.tags == tags
        && last.refused_tags == refused_tags
    {
        last.last = frame.hour;
    } else {
        scan.presence.push(PresenceRun {
            first: frame.hour,
            last: frame.hour,
            tags,
            refused_tags,
        });
    }
    Ok(())
}

pub(super) fn observe_all(
    frame: &DecodedFrame,
    scan: &mut Scanned,
    content: Option<&PoliticsContent>,
) -> io::Result<()> {
    let hour = frame.hour;
    observe_presence(scan, frame)?;
    scan.refresh_modifier_formats(frame);
    let mut event_countries = BTreeSet::new();
    for row in frame
        .complete::<tables::CountryPowerChanges>()
        .unwrap_or_default()
    {
        if !scan
            .events
            .contains(&(row.country, row.instance, row.epoch, row.sequence))
        {
            event_countries.insert(row.country);
            event_countries.extend(row.identity.target_country);
        }
    }
    for row in frame
        .complete::<tables::CountryPowerDailyCosts>()
        .unwrap_or_default()
    {
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
    scan.name_frame(frame, content, &event_countries);
    if let Some(previous) = scan.previous.take() {
        scan.name_frame(&previous, content, &event_countries);
        scan.previous = Some(previous);
    }
    for country in frame.complete::<tables::Countries>().unwrap_or_default() {
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
    let countries: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()
        .unwrap_or_default()
        .iter()
        .filter(|row| valid_recorded_tag(row).is_ok())
        .map(|row| (row.canonical_slot, row))
        .collect();
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .unwrap_or_default()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let witnesses: BTreeMap<_, _> = frame
        .complete::<tables::PoliticsCaptureCoverage>()
        .unwrap_or_default()
        .iter()
        .map(|row| (row.country, row))
        .collect();
    let by_slot: BTreeMap<_, _> = witnesses
        .values()
        .filter_map(|row| links.get(&row.country).map(|slot| (*slot, *row)))
        .collect();
    let mut daily_mutations: BTreeMap<u32, BTreeSet<i32>> = BTreeMap::new();
    let mut unknown_additions: BTreeMap<(u32, u32, u64), u8> = BTreeMap::new();
    for row in frame
        .complete::<tables::CountryPowerChanges>()
        .unwrap_or_default()
    {
        if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political {
            daily_mutations
                .entry(row.country)
                .or_default()
                .insert(row.hour);
        }
        // An Add without its evaluated operand does not establish direction.
        // PP's excluded daily net is instead settled by its decomposition.
        if row.operation == PowerOperation::Add
            && row.requested_delta_raw.is_none()
            && !(row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political)
            && witnesses.get(&row.country).is_some_and(|witness| {
                witness.instance == row.instance
                    && witness.epoch == row.epoch
                    && row.hour >= witness.since_hour
                    && row.hour <= witness.through_hour
            })
        {
            *unknown_additions
                .entry((row.country, row.instance, row.epoch))
                .or_default() |= match row.currency {
                PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
                PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
            };
        }
    }
    for witness in witnesses.values() {
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
    // collector publish_daily preserves the native wrapping sum and
    // matching applied PP sequence. Complete tables alone do not prove that
    // a positive native daily cost supplied its component rows.
    let mut component_sums: BTreeMap<_, i64> = BTreeMap::new();
    for cost in frame
        .complete::<tables::CountryPowerDailyCosts>()
        .unwrap_or_default()
    {
        let key = (
            cost.country,
            cost.instance,
            cost.epoch,
            cost.day_hour,
            cost.pp_sequence,
        );
        let sum = component_sums.entry(key).or_default();
        *sum = sum.wrapping_add(cost.cost_raw);
    }
    let mut applications = BTreeSet::new();
    let mut unreconciled = BTreeSet::new();
    for day in frame
        .complete::<tables::CountryPowerDaily>()
        .unwrap_or_default()
    {
        if let Some(sequence) = day.pp_sequence {
            let key = (day.country, day.instance, day.epoch, day.day_hour, sequence);
            applications.insert(key);
            if day
                .pp_cost_raw
                .is_none_or(|cost| component_sums.get(&key).copied().unwrap_or(0) != cost)
            {
                unreconciled.insert(day.country);
            }
        }
    }
    for event in frame
        .complete::<tables::CountryPowerChanges>()
        .unwrap_or_default()
    {
        if event.cause == PowerCause::DailyGain
            && event.currency == PowerCurrency::Political
            && !scan
                .events
                .contains(&(event.country, event.instance, event.epoch, event.sequence))
            && !applications.contains(&(
                event.country,
                event.instance,
                event.epoch,
                event.hour,
                event.sequence,
            ))
        {
            unreconciled.insert(event.country);
        }
    }
    // Every current country gets a witness or an explicit unknown interval.
    for country in countries.values() {
        let tag = std::str::from_utf8(&country.tag)
            .expect("recorded country tags validated at scan entry");
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
                .get(&(witness.country, witness.instance, witness.epoch))
                .copied()
                .unwrap_or(0)
        });
        let political = value.streams & PoliticsCaptureCoverageRow::PP != 0
            && changes_known
            && daily_known
            && !unreconciled.contains(&raw)
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
                || unreconciled.contains(&raw)
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
        let reply = scan.country(tag, frame.branch);
        merge_coverage(&mut reply.coverage, value);
        merge_spending_coverage(&mut reply.spending_coverage, spending_cover);
    }
    if !unreconciled.is_empty() {
        for country in &unreconciled {
            if let Some(country) = links.get(country).and_then(|slot| countries.get(slot)) {
                scan.note(valid_recorded_tag(country).ok(), "Daily PP rates/components have no matching native application and wrapping sum".into(), 1)?;
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
            for country in countries.values() {
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
    let previous = scan.previous.take();
    let current_world = std::sync::OnceLock::new();
    let previous_world = std::sync::OnceLock::new();
    if let Ok(rows) = frame.complete::<tables::CountryPowerChanges>() {
        for row in rows {
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.hour)
                .map(str::to_owned)
            else {
                continue;
            };
            if !scan
                .events
                .insert((row.country, row.instance, row.epoch, row.sequence))
            {
                continue;
            }
            let bit = match row.currency {
                PowerCurrency::Political => PoliticsCaptureCoverageRow::PP,
                PowerCurrency::Command => PoliticsCaptureCoverageRow::CP,
            };
            let item: LedgerItem = (&row.identity).into();
            let mutation = LedgerMutation {
                country: row.country,
                instance: row.instance,
                sequence: row.sequence.to_string(),
                epoch: row.epoch.to_string(),
                hour: row.hour,
                read_hour: hour,
                currency: format!("{:?}", row.currency),
                cause: format!("{:?}", row.cause),
                operation: format!("{:?}", row.operation),
                site: format!("{:?}", row.site),
                amount_raw: row.requested_delta_raw.map(|raw| raw.to_string()),
                realized_delta_raw: row.realized_delta_raw().to_string(),
                before_raw: row.balance_before_raw.to_string(),
                after_raw: row.balance_after_raw.to_string(),
                item,
                covered: covered(row.country, row.instance, row.epoch, row.hour, bit),
            };
            {
                let event_frame = if row.hour == hour {
                    Some(frame)
                } else {
                    previous
                        .as_ref()
                        .filter(|previous| previous.hour == row.hour)
                };
                let world_cache = if row.hour == hour {
                    &current_world
                } else {
                    &previous_world
                };
                store_facts(scan, &mutation, event_frame, row.cause, None);
                if let Some(content) = content {
                    store_item_text(scan, &mutation, content, event_frame, world_cache);
                }
            }
            scan.country(&tag, frame.branch).mutations.push(mutation);
        }
    }
    if let Ok(rows) = frame.complete::<tables::GovernmentChanges>() {
        for row in rows {
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.hour)
                .map(str::to_owned)
            else {
                continue;
            };
            if !scan
                .events
                .insert((row.country, row.instance, row.epoch, row.sequence))
            {
                continue;
            }
            let before_token =
                government_token(frame, previous.as_ref(), row.hour, row.before_ideology);
            let after_token =
                government_token(frame, previous.as_ref(), row.hour, row.after_ideology);
            scan.country(&tag, frame.branch)
                .governments
                .push(LedgerGovernment {
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
    if let Ok(rows) = frame.complete::<tables::CountryPowerDaily>() {
        for row in rows {
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.day_hour)
                .map(str::to_owned)
            else {
                continue;
            };
            let key = (row.country, row.instance, row.epoch, row.day_hour);
            let mut value = LedgerDaily {
                country: row.country,
                instance: row.instance,
                epoch: row.epoch.to_string(),
                day_hour: row.day_hour,
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
                pp_gross_raw: row.pp_gross_gain_raw.map(|raw| raw.to_string()),
                pp_cost_raw: row.pp_cost_raw.map(|raw| raw.to_string()),
                pp_income_raw: row.pp_net_gain_raw.map(|raw| raw.to_string()),
                pp_realized_raw: row.pp_realized_delta_raw.map(|raw| raw.to_string()),
                pp_sequence: row.pp_sequence.map(|raw| raw.to_string()),
                cp_income_raw: row.cp_gain_raw.map(|raw| raw.to_string()),
                cp_realized_raw: row.cp_realized_delta_raw.map(|raw| raw.to_string()),
                cp_sequence: row.cp_sequence.map(|raw| raw.to_string()),
            };
            if let Some(&index) = scan.daily.get(&key) {
                let old = &mut scan.country(&tag, frame.branch).daily[index];
                // publish_daily commits matching PP rate/application operands
                // together, independently of CP.
                if [
                    value.pp_gross_raw.as_deref(),
                    value.pp_cost_raw.as_deref(),
                    value.pp_income_raw.as_deref(),
                    value.pp_realized_raw.as_deref(),
                    value.pp_sequence.as_deref(),
                ] == [
                    old.pp_gross_raw.as_deref(),
                    old.pp_cost_raw.as_deref(),
                    old.pp_income_raw.as_deref(),
                    old.pp_realized_raw.as_deref(),
                    old.pp_sequence.as_deref(),
                ] {
                    value.pp_read_hour = old.pp_read_hour;
                }
                if [
                    value.cp_income_raw.as_deref(),
                    value.cp_realized_raw.as_deref(),
                    value.cp_sequence.as_deref(),
                ] == [
                    old.cp_income_raw.as_deref(),
                    old.cp_realized_raw.as_deref(),
                    old.cp_sequence.as_deref(),
                ] {
                    value.cp_read_hour = old.cp_read_hour;
                }
                let mut unchanged = value.clone();
                unchanged.read_hour = old.read_hour;
                if *old != unchanged {
                    *old = value;
                }
            } else {
                let index = scan.country(&tag, frame.branch).daily.len();
                scan.daily.insert(key, index);
                scan.country(&tag, frame.branch).daily.push(value);
            }
        }
    }
    if let Ok(rows) = frame.complete::<tables::CountryPowerDailyCosts>() {
        for row in rows {
            let Some(tag) = scan
                .tag_at(row.country, row.instance, row.epoch, row.day_hour)
                .map(str::to_owned)
            else {
                continue;
            };
            if !scan.costs.insert((
                row.country,
                row.instance,
                row.epoch,
                row.day_hour,
                row.ordinal,
            )) {
                continue;
            }
            let cost = LedgerDailyCost {
                country: row.country,
                instance: row.instance,
                epoch: row.epoch.to_string(),
                day_hour: row.day_hour,
                pp_sequence: row.pp_sequence.to_string(),
                ordinal: row.ordinal,
                read_hour: hour,
                covered: covered(
                    row.country,
                    row.instance,
                    row.epoch,
                    row.day_hour,
                    PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::PP_DAILY_COSTS,
                ) && !unreconciled.contains(&row.country),
                component: format!("{:?}", row.component),
                amount_raw: row.cost_raw.to_string(),
                cause: format!("{:?}", row.cause),
                item: (&row.identity).into(),
            };
            {
                let event_frame = if row.day_hour == hour {
                    Some(frame)
                } else {
                    previous
                        .as_ref()
                        .filter(|previous| previous.hour == row.day_hour)
                };
                let world_cache = if row.day_hour == hour {
                    &current_world
                } else {
                    &previous_world
                };
                let mutation = daily_mutation(&cost);
                store_facts(scan, &mutation, event_frame, row.cause, Some(row.component));
                if let Some(content) = content {
                    store_item_text(scan, &mutation, content, event_frame, world_cache);
                }
            }
            scan.country(&tag, frame.branch).daily_costs.push(cost);
        }
    }
    holds::observe_holds(frame, scan, content);
    scan.first_hour.get_or_insert(hour);
    scan.last_hour = Some(scan.last_hour.map_or(hour, |last| last.max(hour)));
    scan.previous = Some(frame.clone());
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
fn world_hours(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<BTreeSet<i32>> {
    let selected = [
        TableId::Campaign,
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
    ];
    let mut hours = BTreeSet::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        cancelled_request(cancelled)?;
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            cancelled_request(cancelled)?;
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, replay)?;
            for row in decoded
                .complete::<tables::CountryPowerChanges>()
                .unwrap_or_default()
            {
                if matches!(
                    row.cause,
                    PowerCause::Decision
                        | PowerCause::TargetedDecision
                        | PowerCause::DecisionTimeout
                        | PowerCause::Focus
                        | PowerCause::Idea
                        | PowerCause::Advisor
                        | PowerCause::AdvisorRemoval
                ) {
                    hours.insert(row.hour);
                }
            }
            for row in decoded
                .complete::<tables::CountryPowerDailyCosts>()
                .unwrap_or_default()
            {
                if row.identity.object_kind == replay::rows::PowerObjectKind::Organisation {
                    hours.insert(row.day_hour);
                }
            }
        }
    }
    Ok(hours)
}
fn scan_branch(
    replay: &Replay,
    branch: u64,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
) -> io::Result<Scanned> {
    // First identify event hours from the ledger. Decode trigger operands only
    // in blocks that contain an exact event hour, including a preceding block
    // whose final hour is consumed by the next read's handoff.
    let needed = if content.is_some() {
        world_hours(replay, branch, cancelled)?
    } else {
        BTreeSet::new()
    };
    let mut selected = vec![
        TableId::Campaign,
        TableId::CountryPowerDaily,
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
        TableId::GovernmentChanges,
        TableId::PoliticsCaptureCoverage,
        TableId::CharacterPortraits,
        TableId::AdvisorRoles,
        TableId::Characters,
        TableId::UnitLeaders,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ModifierDefinitions,
        TableId::CountryAppearance,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
        TableId::CountryCivilWars,
        TableId::CountryCounterInputs,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::CountryCommandPower,
        TableId::IndustrialOrganisations,
    ];
    selected.sort_unstable();
    selected.dedup();
    let mut with_world = selected.clone();
    with_world.extend_from_slice(crate::triggers::RECORDED_TABLES);
    with_world.sort_unstable();
    with_world.dedup();
    let mut scan = Scanned {
        first_hour: replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.first_hour)
            .min(),
        last_hour: replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.last_hour)
            .max(),
        ..Default::default()
    };
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        cancelled_request(cancelled)?;
        let tables = if needed
            .range(block.first_hour..=block.last_hour)
            .next()
            .is_some()
        {
            &with_world
        } else {
            &selected
        };
        let frames = match replay.read_block_selected(block, tables) {
            Ok(frames) => frames,
            Err(_) => {
                scan.note(None, "A recording block could not be read".into(), 1)?;
                continue;
            }
        };
        let mut decoder = FrameDecoder::for_tables(tables);
        for frame in frames {
            cancelled_request(cancelled)?;
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, replay)?;
            observe_all(decoded, &mut scan, content)?;
        }
    }
    scan.previous = None;
    Ok(scan)
}

pub(super) fn load(
    source: &Source,
    params: &PoliticsLedgerParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<PoliticsLedgerReply> {
    let content = context
        .content
        .as_ref()
        .map(|options| politics_content(source, options))
        .transpose()?;
    let catalogue = content
        .as_ref()
        .map(|content| Arc::clone(&content.catalogue));
    let state = source.state::<Cache>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let same = match (&state.catalogue, &catalogue) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    };
    if state.branch != Some(params.branch()) || !same || state.scan.is_none() {
        state.scan = Some(Arc::new(scan_branch(
            &source.replay(),
            params.branch(),
            content.as_deref(),
            cancelled,
        )?));
        state.branch = Some(params.branch());
        state.catalogue = catalogue;
    }
    let scan = Arc::clone(state.scan.as_ref().expect("scan loaded"));
    drop(state);
    present_view(
        &scan,
        params.branch(),
        &params.tag,
        content.as_deref(),
        cancelled,
        params.view.clone(),
    )
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

fn recorded_token<'a>(scan: &'a Scanned, row: &'a LedgerMutation) -> Option<&'a str> {
    scan.event_facts
        .get(&item_text_id(row))
        .and_then(|facts| facts.token.as_deref())
        .or_else(|| {
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
fn item_text_id(row: &LedgerMutation) -> String {
    if let Some(ordinal) = row.site.strip_prefix("daily component:") {
        format!(
            "daily:{}:{}:{}:{}:{ordinal}",
            row.country, row.instance, row.epoch, row.hour
        )
    } else {
        mutation_id(row)
    }
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
fn store_item_text<'a>(
    scan: &mut Scanned,
    row: &LedgerMutation,
    content: &'a PoliticsContent,
    frame: Option<&'a DecodedFrame>,
    world_cache: &std::sync::OnceLock<Result<crate::triggers::World<'a>, String>>,
) {
    if let Some(mut text) = event_text(scan, row, content, frame, world_cache) {
        text.unknown = text.unknown.map(|reason| undated_reason(&reason));
        scan.event_text.insert(item_text_id(row), text);
    }
}

fn localise_at(
    content: &PoliticsContent,
    key: &str,
    frame: Option<&DecodedFrame>,
    slot: Option<u32>,
    decision: Option<(&LedgerMutation, bool)>,
    unknown_seed: bool,
) -> Result<Option<String>, String> {
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
        let scripted = crate::scripted_text::ScriptedText::new(
            frame,
            &content.catalogue.localisation,
            content.catalogue.scripted_content()?,
            slot,
        );
        if let Some((decision, selects_state)) = decision {
            use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
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
            use crate::scripted_text::{EventObject, EventScope, EventScopes};
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
) -> Result<crate::triggers::World<'a>, String> {
    let full = content
        .catalogue
        .scripted_content()
        .and_then(|scripted| scripted.recorded_world(frame));
    match full {
        Ok(world) => Ok(world),
        Err(_) => {
            let power = content.power_content()?;
            Ok(crate::triggers::World::read(
                frame,
                crate::triggers::ContentFacts::content_only(
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
    content: &'a PoliticsContent,
    frame: Option<&'a DecodedFrame>,
    world_cache: &std::sync::OnceLock<Result<crate::triggers::World<'a>, String>>,
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
            let scripted = crate::scripted_text::ScriptedText::new(
                frame,
                &content.catalogue.localisation,
                content.catalogue.scripted_content()?,
                own.map_or(0, |own| own.naming.slot),
            );
            // The helper supplies the native organisation/owner pair; the
            // country's default expansion scope is not used for this name.
            scripted
                .organisation_name(reference)
                .and_then(|text| plain_text(&text))
        })();
        match result {
            Ok(name) => found.name = Some(name),
            Err(reason) => found.unknown = Some(reason),
        }
        return Some(found);
    }
    let token = recorded_token(scan, row)?;
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
                    world_cache
                        .get_or_init(|| event_world(content, frame))
                        .as_ref()
                        .map_err(Clone::clone)?
                        .has_children(body)
                })?;
                localise_at(
                    content,
                    &key,
                    frame,
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
                let world = world_cache
                    .get_or_init(|| event_world(content, frame))
                    .as_ref()
                    .map_err(Clone::clone)?;
                let owner = crate::triggers::Scope::Country(own.naming.slot);
                let mut scopes = crate::triggers::Scopes::of(owner);
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
                        (Some(raw), _) => world.slot(raw).map(crate::triggers::Scope::Country),
                        (None, Some(state)) => Some(crate::triggers::Scope::State(state)),
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
                        let world = world_cache
                            .get_or_init(|| event_world(content, frame))
                            .as_ref()
                            .map_err(Clone::clone)?;
                        let owner = crate::triggers::Scope::Country(own.naming.slot);
                        let mut scopes = crate::triggers::Scopes::of(owner);
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
        if row.cause == "Idea" {
            match content.idea_name(token, own.naming.for_tag(&own.tag)) {
                Ok(crate::products::politics_content::IdeaName::Key(key)) => {
                    match localise_at(content, &key, frame, Some(own.naming.slot), None, false) {
                        Ok(name) => found.name = name,
                        Err(reason) => found.unknown = Some(reason),
                    }
                }
                Ok(crate::products::politics_content::IdeaName::Generated(name)) => {
                    found.name = Some(name)
                }
                Err(reason) => found.unknown = Some(reason),
            }
        }
        found.description = content.idea_description_key(token).and_then(|key| {
            match localise_at(content, &key, frame, Some(own.naming.slot), None, false) {
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
fn sprite_png(content: &PoliticsContent, name: &str) -> Result<Option<String>, String> {
    content.sprite_png(name).map_err(|error| error.to_string())
}
fn character_art(
    content: &PoliticsContent,
    recorded: &facts::CharacterFact,
) -> (Option<String>, Option<String>, Option<String>) {
    let name = facts::character_name(recorded, content);
    match crate::products::character_portraits::portrait_png(content, &recorded.portrait) {
        Ok(icon) => (name, Some(icon), None),
        Err(reason) => (
            name,
            None,
            Some(format!("character {} portrait: {reason}", recorded.id)),
        ),
    }
}
fn resolve_item(
    scan: &Scanned,
    row: &LedgerMutation,
    content: Option<&PoliticsContent>,
    id: String,
    daily_component: Option<&str>,
) -> (LedgerSpendItem, Option<IdeaInfo>, (String, String)) {
    let own = scan.name_at(row.country, row.hour);
    let target = row
        .item
        .target_country
        .and_then(|raw| scan.name_at(raw, row.hour));
    let facts = scan.event_facts.get(&item_text_id(row));
    let token = recorded_token(scan, row).map(str::to_owned);
    let recorded = facts.and_then(|facts| facts.character.as_ref());
    let character = row
        .item
        .character
        .or_else(|| recorded.map(|recorded| recorded.id));
    let (category_id, category_name) =
        category(facts.map_or(PowerCause::NotIdentified, |facts| facts.native_cause));
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
                                match sprite_png(content, sprite) {
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
        if let Some(text) = scan.event_text.get(&item_text_id(row)) {
            if !matches!(row.cause.as_str(), "Advisor" | "AdvisorRemoval") {
                item.name = text.name.clone();
            }
            combine_reason(&mut item.unknown, text.unknown.clone());
            if let Some(sprite) = text.sprite.as_deref() {
                match sprite_png(content, sprite) {
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
                        Ok(Some(sprite)) => match sprite_png(content, &sprite) {
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
                if let Some(modifier) = facts.and_then(|facts| facts.modifier.as_ref()) {
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
                            match crate::catalogue::localised_with(
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
                if let Some(organisation) = facts.and_then(|facts| facts.organisation.as_ref()) {
                    // The scoped name is frozen by event_text at this read. Only artwork
                    // is loaded at presentation. Reuse the production kit's recorded
                    // owner/key precedence and icon fallback.
                    let sprite = crate::products::production_lines::organisation_icon_with_token(
                        organisation,
                        facts.is_some_and(|facts| facts.organisation_token_known),
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
                        Ok(Some(sprite)) => match sprite_png(content, &sprite) {
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
            scan.event_text
                .get(&item_text_id(row))
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

fn present_view(
    scan: &Scanned,
    branch: u64,
    tag: &str,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
    view: LedgerView,
) -> io::Result<PoliticsLedgerReply> {
    let mut budget = ReplyBudget::new();
    let empty = PoliticsLedgerReply {
        branch_id: branch.to_string(),
        tag: tag.into(),
        decimals: 5,
        ..Default::default()
    };
    let raw = scan.countries.get(tag).unwrap_or(&empty);
    if view == LedgerView::Raw {
        budget.add(raw)?;
    }
    let mut reply = if view == LedgerView::Governments {
        empty.clone()
    } else if view != LedgerView::Raw {
        budget.add(&raw.spending_coverage)?;
        if view == LedgerView::Balance {
            budget.add(&raw.daily)?;
        }
        PoliticsLedgerReply {
            spending_coverage: raw.spending_coverage.clone(),
            daily: if view == LedgerView::Balance {
                raw.daily.clone()
            } else {
                Vec::new()
            },
            ..empty.clone()
        }
    } else {
        raw.clone()
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
    for row in &raw.mutations {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Politics request cancelled",
            ));
        }
        let Some(charge) = charge(row) else {
            continue;
        };
        let id = format!(
            "{}:{}:{}:{}",
            row.country, row.instance, row.epoch, row.sequence
        );
        let (mut item, mut info, category) = resolve_item(scan, row, content, id.clone(), None);
        intern_tooltip(&mut item, &mut info, &mut tooltip_ids, tag);
        if let Some(reason) = &item.unknown {
            count_reason(&mut reasons, reason.clone(), 1)?;
        }
        if let Some(info) = info
            && info_ids.insert(info.token.clone())
        {
            let artwork = crate::Artwork {
                token: info.token.clone(),
                name: item.name.clone(),
                description: scan
                    .event_text
                    .get(&item_text_id(row))
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
            day.pp_sequence.as_deref().map(|sequence| {
                (
                    day.country,
                    day.instance,
                    day.epoch.as_str(),
                    day.day_hour,
                    sequence,
                )
            })
        })
        .collect();
    for cost in &raw.daily_costs {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Politics request cancelled",
            ));
        }
        let Some(amount) = cost
            .amount_raw
            .parse::<i128>()
            .ok()
            .filter(|amount| *amount != 0)
        else {
            continue;
        };
        if !daily_applications.contains(&(
            cost.country,
            cost.instance,
            cost.epoch.as_str(),
            cost.day_hour,
            cost.pp_sequence.as_str(),
        )) {
            invalidate_read(
                &mut reply.spending_coverage,
                cost.read_hour,
                "Daily political power costs have no matched payment at this read",
            );
            count_reason(
                &mut reasons,
                "Daily political power cost: matched PP application is unavailable".into(),
                1,
            )?;
            continue;
        }
        let id = format!(
            "daily:{}:{}:{}:{}:{}",
            cost.country, cost.instance, cost.epoch, cost.day_hour, cost.ordinal
        );
        let row = daily_mutation(cost);
        let (mut item, mut info, mut category) =
            resolve_item(scan, &row, content, id.clone(), Some(&cost.component));
        if amount < 0 {
            let name = item.name.as_deref().unwrap_or("Not identified");
            count_reason(
                &mut reasons,
                format!(
                    "Negative daily political power cost component {name}: recorded credit is not included in spending"
                ),
                1,
            )?;
            invalidate_read(
                &mut reply.spending_coverage,
                cost.read_hour,
                "Negative daily political power cost components are not included in spending",
            );
            continue;
        }
        intern_tooltip(&mut item, &mut info, &mut tooltip_ids, tag);
        if let Some(reason) = &item.unknown {
            count_reason(&mut reasons, reason.clone(), 1)?;
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
            let artwork = crate::Artwork {
                token: info.token.clone(),
                name: item.name.clone(),
                description: scan
                    .event_text
                    .get(&item_text_id(&row))
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
    if view == LedgerView::Balance {
        for row in &raw.mutations {
            cancelled_request(cancelled)?;
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
                _ => {
                    // Shares spending's dated item/content resolution. The chart
                    // needs the sentence; the log owns art and idea tooltips.
                    let (item, _, _) = resolve_item(scan, row, content, mutation_id(row), None);
                    (item.action, item.unknown)
                }
            };
            if let Some(reason) = &reason
                && charge(row).is_none()
            {
                count_reason(&mut reasons, reason.clone(), 1)?;
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
